/*
Parallel run construction. A single producer assigns doc IDs and sends
bounded batches to workers. Each worker owns its posting buffer.
Page records are written at doc_id * 8, so completion order cannot
change the mapping between internal doc IDs and original passage IDs.
*/
use std::fs::{self, File};
use std::io::{self, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;

use super::{build_runs, builder::IndexBuilder, reader};
use crate::tokenizer::Tokenizer;

const WORK_BYTES: usize = 64 * 1024;
const WORK_DOCUMENTS: usize = 128;
type Batch = Vec<(u32, u32, String)>; // internal ID, passage ID, text

pub fn build_runs_parallel<T: Tokenizer + Sync>(
    input_path: &str,
    output_path: &str,
    max_batch_bytes: usize,
    tokenizer: &T,
    workers: usize,
) -> io::Result<usize> {
    if workers == 0 || max_batch_bytes == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Workers and batch bytes must be positive",
        ));
    }
    if workers == 1 {
        return build_runs(input_path, output_path, max_batch_bytes, tokenizer);
    }

    fs::create_dir_all(output_path)?;
    let output = Path::new(output_path);
    let page_table = Mutex::new(File::create(output.join("page_table.bin"))?);
    let run_count = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let (sender, receiver) = mpsc::sync_channel::<Batch>(workers);
    let receiver = Arc::new(Mutex::new(receiver));

    thread::scope(|scope| {
        let mut handles = Vec::new();
        for _ in 0..workers {
            let receiver = Arc::clone(&receiver);
            let page_table = &page_table;
            let run_count = &run_count;
            let cancelled = &cancelled;
            handles.push(scope.spawn(move || {
                let result = run_worker(
                    receiver,
                    page_table,
                    run_count,
                    cancelled,
                    output,
                    max_batch_bytes,
                    tokenizer,
                );
                if result.is_err() {
                    cancelled.store(true, Ordering::Relaxed);
                }
                result
            }));
        }
        // If all workers exit, a blocked producer must see disconnection.
        drop(receiver);
        let producer_result = send_batches(input_path, sender, &cancelled);
        let mut worker_error = None;
        for handle in handles {
            let result = handle.join().unwrap_or_else(|_| {
                cancelled.store(true, Ordering::Relaxed);
                Err(io::Error::other("Index worker panicked"))
            });
            if let Err(error) = result {
                worker_error.get_or_insert(error);
            }
        }
        if let Some(error) = worker_error {
            return Err(error);
        }
        producer_result?;
        Ok(run_count.load(Ordering::Relaxed))
    })
}

fn send_batches(
    input_path: &str,
    sender: mpsc::SyncSender<Batch>,
    cancelled: &AtomicBool,
) -> io::Result<()> {
    let mut batch = Vec::new();
    let mut bytes = 0;
    for (number, passage) in reader::read_passages_from_file(input_path)?.enumerate() {
        if cancelled.load(Ordering::Relaxed) {
            return Err(io::Error::other("Run construction cancelled"));
        }
        let (passage_id, text) = passage?;
        let doc_id = u32::try_from(number)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Too many documents"))?;
        bytes += text.len() + 8;
        batch.push((doc_id, passage_id, text));
        if bytes >= WORK_BYTES || batch.len() >= WORK_DOCUMENTS {
            sender
                .send(std::mem::take(&mut batch))
                .map_err(|_| io::Error::other("Index workers stopped"))?;
            bytes = 0;
        }
    }
    if !batch.is_empty() {
        sender
            .send(batch)
            .map_err(|_| io::Error::other("Index workers stopped"))?;
    }
    Ok(()) // Dropping sender lets idle workers finish.
}

fn run_worker<T: Tokenizer>(
    receiver: Arc<Mutex<mpsc::Receiver<Batch>>>,
    page_table: &Mutex<File>,
    run_count: &AtomicUsize,
    cancelled: &AtomicBool,
    output: &Path,
    max_batch_bytes: usize,
    tokenizer: &T,
) -> io::Result<()> {
    let mut builder = IndexBuilder::new();
    while !cancelled.load(Ordering::Relaxed) {
        let batch = {
            let receiver = receiver
                .lock()
                .map_err(|_| io::Error::other("Work queue poisoned"))?;
            receiver.recv()
        };
        let Ok(batch) = batch else { break };
        if cancelled.load(Ordering::Relaxed) {
            break;
        }
        let first_id = batch[0].0;
        let mut metadata = Vec::with_capacity(batch.len() * 8);
        for (doc_id, passage_id, text) in batch {
            let doc_length = builder.add_document(doc_id, tokenizer.tokenize(&text));
            metadata.extend_from_slice(&passage_id.to_le_bytes());
            metadata.extend_from_slice(&doc_length.to_le_bytes());
            if builder.buffered_bytes >= max_batch_bytes {
                flush_run(&mut builder, output, run_count)?;
            }
        }
        // One lock and one contiguous write per input batch, not per term.
        let mut page = page_table
            .lock()
            .map_err(|_| io::Error::other("Page table poisoned"))?;
        page.seek(SeekFrom::Start(u64::from(first_id) * 8))?;
        page.write_all(&metadata)?;
    }
    if !cancelled.load(Ordering::Relaxed) && builder.postings_count() > 0 {
        flush_run(&mut builder, output, run_count)?;
    }
    Ok(())
}

fn flush_run(builder: &mut IndexBuilder, output: &Path, counter: &AtomicUsize) -> io::Result<()> {
    let number = counter.fetch_add(1, Ordering::Relaxed) + 1;
    let path = output.join(format!("run_{number:04}.bin"));
    let path = path
        .to_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Output path must be UTF-8"))?;
    builder.flush_to_file(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokenizer::SimpleTokenizer;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn worker_write_error_is_returned_without_blocking_the_producer() {
        let name = format!(
            "run-worker-error-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(name);
        fs::create_dir(&root).unwrap();
        let input = root.join("input.tsv");
        fs::write(
            &input,
            (0..1000)
                .map(|i| format!("{i}\trust search\n"))
                .collect::<String>(),
        )
        .unwrap();
        let output = root.join("runs");
        fs::create_dir(&output).unwrap();
        fs::create_dir(output.join("run_0001.bin")).unwrap();
        let result = build_runs_parallel(
            input.to_str().unwrap(),
            output.to_str().unwrap(),
            1,
            &SimpleTokenizer,
            4,
        );
        assert!(result.is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
