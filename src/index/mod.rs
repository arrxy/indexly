mod lexicon;
mod inverted_index;
mod page_table;

pub(crate) use lexicon::Lexicon;

/*
    This module contains the data structures for the search engine's inverted index.
    doc 0: This is a test document.
    doc 1: This is another test document.
    doc 2: This is a third test document.

    Inverted index structure:
    term : (doc_id, term_freq)

    This : (0, 1), (1, 1), (2, 1)
    is : (0, 1), (1, 1), (2, 1)
    a : (0, 1), (1, 1), (2, 1)
    test : (0, 1), (1, 1), (2, 1)
    document : (0, 1), (1, 1), (2, 1)
    another : (1, 1)
    third : (2, 1)

    We have 3 data structures:

    1. Lexicon:

        pub struct Lexicon {
            pub terms: HashMap<String, LexiconEntry>,
        }

        pub struct LexiconEntry {
            pub offset: usize,
            pub length: usize,
            pub doc_freq: usize,
        }

        Lexicon is a lookup table that maps terms to their corresponding entry.
        Each entry contains the offset, length and document frequency of the term in the inverted index file.
        Example "test": LexiconEntry { offset: 0, length: 12, doc_freq: 3 }
        length is the number of bytes in the inverted index file for this term, and doc_freq is
        the number of documents that contain this term.

    2. Inverted Index:

        pub struct InvertedIndex {
            pub lexicon: Lexicon,
            pub postings_file: File
        }

        InvertedIndex connects terms to the documents that contain them.
        The lexicon maps each term to the location of its posting list.
        The postings_file stores the actual document IDs and term
        frequencies.

        For example, to search for "test", we find its offset and length
        in the lexicon, then read its posting list from postings_file:

            "test" -> (0, 1), (1, 1), (2, 1)

        Each pair is (doc_id, term_freq). The postings stay on disk;
        the InvertedIndex struct does not hold every list in memory.

        struct DecodedBlock {
            doc_ids: Vec<u32>,
            frequencies: Vec<u32>,
        }

        This struct represents a block of postings that has been decoded from the postings file.

        struct BlockHeader {
            first_doc_id: u32,
            last_doc_id: u32,
            posting_count: u32,
            doc_gaps_bytes: u32,
            frequencies_bytes: u32,
        }

        Actual postings are stored in blocks,
        each with a header that describes the block's contents. The header includes the first and last document IDs,
        the number of postings, and the number of bytes used to store document gaps and frequencies.

    3. Page Table:
        pub struct PageTable {
            pub file: File,
            pub entries: Vec<PageTableEntry>,
        }

        pub struct PageTableEntry {
            pub offset: u64,
            pub length: u32,
            pub doc_length: u32,
        }

        The page table maps each internal doc_id to its original passage ID.
        For sequential doc IDs, entries[doc_id] gives that document's metadata.

        Each entry's offset and length locate the original passage ID in the
        page table file. doc_length is the number of indexed terms in the
        passage; it is used when calculating BM25.
*/