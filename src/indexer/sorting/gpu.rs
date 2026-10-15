/* GPU radix sorting through wgpu's Metal backend. A shared context serializes
GPU submissions while CPU workers prepare their own batch dictionaries.
Only numeric keys and original posting indices go to the GPU; strings stay
on the CPU. Run files keep their existing term-text binary format. */
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::io;
use std::sync::{Arc, Mutex, mpsc};
use std::task::{Context, Poll, Wake, Waker};
use std::thread;
use std::time::{Duration, Instant};

use wgpu::util::DeviceExt;

use super::{IntermediatePosting, PostingOrder, PostingSorter};

const THREADS: u32 = 256;
type NumericPosting = [u32; 4]; // term ID, doc ID, original index, reserved

#[derive(Default)]
struct Stats {
    batches: u64,
    postings: u64,
    prepare: Duration,
    wait: Duration,
    gpu: Duration,
}

pub struct GpuSorter {
    context: Mutex<GpuContext>,
    description: String,
    stats: Mutex<Stats>,
}

impl GpuSorter {
    pub fn new() -> io::Result<Self> {
        let context = GpuContext::new()?;
        let description = format!(
            "gpu ({} via Metal, radix sort)",
            context.device.adapter_info().name
        );
        Ok(Self {
            context: Mutex::new(context),
            description,
            stats: Mutex::new(Stats::default()),
        })
    }
}

impl PostingSorter for GpuSorter {
    fn sort(&self, postings: &mut [IntermediatePosting]) -> io::Result<PostingOrder> {
        if postings.is_empty() {
            return Ok(PostingOrder::Indices(Vec::new()));
        }
        let start = Instant::now();
        let records = numeric_postings(postings)?;
        let prepare = start.elapsed();
        let start = Instant::now();
        let context = self
            .context
            .lock()
            .map_err(|_| io::Error::other("GPU sorter poisoned"))?;
        let wait = start.elapsed();
        let start = Instant::now();
        let indices = context.sort_indices(&records)?;
        let gpu = start.elapsed();
        drop(context);
        drop(records);
        let mut stats = self
            .stats
            .lock()
            .map_err(|_| io::Error::other("GPU sort statistics poisoned"))?;
        stats.batches += 1;
        stats.postings += postings.len() as u64;
        stats.prepare += prepare;
        stats.wait += wait;
        stats.gpu += gpu;
        Ok(PostingOrder::Indices(indices))
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn report(&self) {
        if let Ok(stats) = self.stats.lock() {
            println!(
                "Sort GPU: {} batches, {} postings; aggregate worker times: term IDs {:.2}s, GPU queue wait {:.2}s, GPU submission/readback/validation {:.2}s; writer follows sorted indices (no posting reorder)",
                stats.batches,
                stats.postings,
                stats.prepare.as_secs_f64(),
                stats.wait.as_secs_f64(),
                stats.gpu.as_secs_f64()
            );
        }
    }
}

// IDs are ranks in the UNIQUE term vocabulary of this batch. Hash IDs or
// encounter-order IDs would change the ordering expected by the merger.
fn numeric_postings(postings: &[IntermediatePosting]) -> io::Result<Vec<NumericPosting>> {
    if postings.len() > u32::MAX as usize {
        return Err(invalid("GPU batch has more than u32::MAX postings"));
    }
    let mut unique: HashSet<&str> = HashSet::new();
    for posting in postings {
        unique.insert(posting.term.as_str());
    }
    let mut vocabulary: Vec<_> = unique.into_iter().collect();
    vocabulary.sort_unstable();
    let ids: HashMap<_, _> = vocabulary
        .into_iter()
        .enumerate()
        .map(|(id, term)| (term, id as u32))
        .collect();
    Ok(postings
        .iter()
        .enumerate()
        .map(|(index, posting)| [ids[posting.term.as_str()], posting.doc_id, index as u32, 0])
        .collect())
}

struct GpuContext {
    device: wgpu::Device,
    queue: wgpu::Queue,
    layout: wgpu::BindGroupLayout,
    local: wgpu::ComputePipeline,
    prefix: wgpu::ComputePipeline,
    scatter: wgpu::ComputePipeline,
}

impl GpuContext {
    fn new() -> io::Result<Self> {
        if !cfg!(target_os = "macos") {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "This GPU sorting experiment requires macOS Metal",
            ));
        }
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
        }))
        .map_err(|e| io::Error::other(format!("No Metal GPU adapter: {e}")))?;
        if adapter.get_info().device_type == wgpu::DeviceType::Cpu {
            return Err(io::Error::other("GPU sorting requires a hardware GPU"));
        }
        let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("Posting radix sorter"),
            ..Default::default()
        }))
        .map_err(|e| io::Error::other(format!("Could not initialize GPU: {e}")))?;
        let errors = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let entries: Vec<_> = (0..6)
            .map(|binding| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: if binding == 5 {
                        wgpu::BufferBindingType::Uniform
                    } else {
                        wgpu::BufferBindingType::Storage {
                            read_only: binding == 0,
                        }
                    },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            })
            .collect();
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Radix buffers"),
            entries: &entries,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Radix layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Posting radix shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("radix.wgsl").into()),
        });
        let pipeline = |entry| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let local = pipeline("local_sort");
        let prefix = pipeline("prefix");
        let scatter = pipeline("scatter");
        if let Some(error) = block_on(errors.pop()) {
            return Err(io::Error::other(format!(
                "GPU radix pipeline validation failed: {error}"
            )));
        }
        Ok(Self {
            device,
            queue,
            layout,
            local,
            prefix,
            scatter,
        })
    }

    fn sort_indices(&self, records: &[NumericPosting]) -> io::Result<Vec<u32>> {
        if records.is_empty() {
            return Ok(Vec::new());
        }
        let count = u32::try_from(records.len()).map_err(|_| invalid("GPU batch too large"))?;
        let groups = count.div_ceil(THREADS);
        let bytes = std::mem::size_of_val(records) as u64;
        let limits = self.device.limits();
        if bytes > u64::from(limits.max_storage_buffer_binding_size)
            || bytes > limits.max_buffer_size
            || groups > limits.max_compute_workgroups_per_dimension
        {
            return Err(invalid(
                "GPU batch exceeds device buffer or dispatch limits; reduce --batch-bytes",
            ));
        }
        let a = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Numeric posting keys"),
                contents: bytemuck::cast_slice(records),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            });
        let storage = |label, size| {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            })
        };
        let b = storage("Radix output", bytes);
        let local = storage("Locally sorted records", bytes);
        let histogram = storage("Radix bucket counts and starts", u64::from(groups) * 16 * 8);
        let offsets = storage("Radix bucket offsets", u64::from(groups) * 16 * 4);
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Sorted posting readback"),
            size: bytes,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Posting radix sort"),
            });
        let mut input = &a;
        let mut output = &b;
        // LSD order: doc_id is the secondary key, term_id the primary key.
        // Leading zero nibbles need no pass. Always execute one pass per key.
        for component in [1, 0] {
            let maximum = records.iter().map(|r| r[component]).max().unwrap();
            let bits = (32 - maximum.leading_zeros()).max(1);
            for shift in (0..bits).step_by(4) {
                let params = self
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("Radix pass parameters"),
                        contents: bytemuck::cast_slice(&[count, groups, shift, component as u32]),
                        usage: wgpu::BufferUsages::UNIFORM,
                    });
                let buffers = [input, output, &local, &histogram, &offsets, &params];
                let entries: Vec<_> = buffers
                    .iter()
                    .enumerate()
                    .map(|(binding, buffer)| wgpu::BindGroupEntry {
                        binding: binding as u32,
                        resource: buffer.as_entire_binding(),
                    })
                    .collect();
                let bindings = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Radix pass buffers"),
                    layout: &self.layout,
                    entries: &entries,
                });
                for (pipeline, workgroups) in [
                    (&self.local, groups),
                    (&self.prefix, 1),
                    (&self.scatter, groups),
                ] {
                    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("Radix stage"),
                        timestamp_writes: None,
                    });
                    pass.set_pipeline(pipeline);
                    pass.set_bind_group(0, &bindings, &[]);
                    pass.dispatch_workgroups(workgroups, 1, 1);
                }
                std::mem::swap(&mut input, &mut output);
            }
        }
        encoder.copy_buffer_to_buffer(input, 0, &readback, 0, bytes);
        let submission = self.queue.submit([encoder.finish()]);
        let slice = readback.slice(..);
        let (sender, receiver) = mpsc::sync_channel(1);
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(Duration::from_secs(60)),
            })
            .map_err(|e| io::Error::other(format!("GPU sort wait failed: {e}")))?;
        receiver
            .recv_timeout(Duration::from_secs(5))
            .map_err(|e| io::Error::other(format!("GPU readback callback failed: {e}")))?
            .map_err(|e| io::Error::other(format!("GPU readback failed: {e}")))?;
        let view = slice.get_mapped_range();
        let sorted: &[NumericPosting] = bytemuck::cast_slice(&view);
        // Validate ordering and that no record was lost, duplicated or changed
        // before returning the order to the binary run writer.
        let mut seen = vec![false; records.len()];
        let mut indices = Vec::with_capacity(records.len());
        let mut previous = None;
        for record in sorted {
            let old = record[2] as usize;
            let key = (record[0], record[1]);
            if old >= records.len()
                || seen[old]
                || *record != records[old]
                || previous.is_some_and(|p| p > key)
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "GPU returned invalid posting order or permutation",
                ));
            }
            previous = Some(key);
            seen[old] = true;
            indices.push(record[2]);
        }
        drop(view);
        readback.unmap();
        Ok(indices)
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

// Native adapter/device creation returns futures; this small executor needs
// no async runtime and parks until the future's waker signals readiness.
fn block_on<F: Future>(future: F) -> F::Output {
    struct ThreadWake(thread::Thread);
    impl Wake for ThreadWake {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
        fn wake_by_ref(self: &Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(ThreadWake(thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => return value,
            Poll::Pending => thread::park(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn term_ids_preserve_utf8_lexicographic_order() {
        let postings = vec![
            IntermediatePosting {
                term: "é".into(),
                doc_id: 7,
                term_frequency: 1,
            },
            IntermediatePosting {
                term: "z".into(),
                doc_id: u32::MAX,
                term_frequency: 2,
            },
            IntermediatePosting {
                term: "é".into(),
                doc_id: 0,
                term_frequency: 3,
            },
            IntermediatePosting {
                term: "a".into(),
                doc_id: 8,
                term_frequency: 4,
            },
        ];
        let records = numeric_postings(&postings).unwrap();
        assert_eq!(
            records,
            vec![
                [2, 7, 0, 0],
                [1, u32::MAX, 1, 0],
                [2, 0, 2, 0],
                [0, 8, 3, 0]
            ]
        );
    }

    #[test]
    #[ignore = "Requires a hardware Metal GPU; run with --features gpu-sort -- --ignored"]
    fn gpu_radix_matches_cpu_oracle_at_block_boundaries_and_full_u32_keys() {
        let context = GpuContext::new().unwrap();
        for count in [0, 1, 2, 15, 255, 256, 257, 1025, 65537] {
            let mut state = 0x1234_5678_u32;
            let mut records: Vec<NumericPosting> = (0..count)
                .map(|index| {
                    state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                    let term = if index % 7 == 0 { u32::MAX } else { state % 33 };
                    state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                    [term, if index % 13 == 0 { 0 } else { state }, index, 0]
                })
                .collect();
            for shape in 0..3 {
                if shape == 1 {
                    for record in &mut records {
                        record[0] = 0;
                        record[1] = 0;
                    }
                } else if shape == 2 {
                    for (i, record) in records.iter_mut().enumerate() {
                        record[0] = (count as usize - i) as u32;
                        record[1] = u32::MAX;
                    }
                }
                let mut expected: Vec<_> = (0..records.len()).collect();
                expected.sort_by_key(|&i| (records[i][0], records[i][1]));
                let indices = context.sort_indices(&records).unwrap();
                assert_eq!(
                    indices
                        .iter()
                        .map(|&index| index as usize)
                        .collect::<Vec<_>>(),
                    expected,
                    "count={count}, shape={shape}"
                );
            }
        }
        let sorter = GpuSorter::new().unwrap();
        let mut postings: Vec<_> = (0..1031)
            .rev()
            .map(|i| IntermediatePosting {
                term: ["rust", "é", "a", "z", "東京"][i % 5].into(),
                doc_id: i as u32,
                term_frequency: i as u32 % 11 + 1,
            })
            .collect();
        let mut expected = postings.clone();
        expected.sort_by(|a, b| a.term.cmp(&b.term).then(a.doc_id.cmp(&b.doc_id)));
        let original = postings.clone();
        let PostingOrder::Indices(indices) = sorter.sort(&mut postings).unwrap() else {
            panic!("GPU must return sorted indices");
        };
        for (a, b) in indices.iter().map(|&i| &postings[i as usize]).zip(expected) {
            assert_eq!(
                (&a.term, a.doc_id, a.term_frequency),
                (&b.term, b.doc_id, b.term_frequency)
            );
        }
        for (a, b) in postings.iter().zip(&original) {
            assert_eq!(
                (&a.term, a.doc_id, a.term_frequency),
                (&b.term, b.doc_id, b.term_frequency)
            );
        }
        assert!(
            matches!(sorter.sort(&mut []).unwrap(), PostingOrder::Indices(indices) if indices.is_empty())
        );
    }
}
