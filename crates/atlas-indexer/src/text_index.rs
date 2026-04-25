//! Tantivy-backed full-text index.

use crate::{Document, IndexError, Result, SearchResult};
use atlas_core::Hash;
use std::collections::HashMap;
use std::path::Path;
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::{Field, Schema, TextFieldIndexing, TextOptions, FAST, STORED, STRING};
use tantivy::{Index, IndexWriter, ReloadPolicy, TantivyDocument};

pub struct TextIndex {
    index: Index,
    writer: IndexWriter,
    schema: IndexSchema,
}

struct IndexSchema {
    hash_hex: Field,
    path: Field,
    text: Field,
    xattrs_json: Field,
    model_version: Field,
}

fn build_schema() -> (Schema, IndexSchema) {
    let mut builder = Schema::builder();
    let text_opts = TextOptions::default()
        .set_indexing_options(
            TextFieldIndexing::default()
                .set_tokenizer("en_stem")
                .set_index_option(tantivy::schema::IndexRecordOption::WithFreqsAndPositions),
        )
        .set_stored();

    let hash_hex = builder.add_text_field("hash_hex", STRING | STORED);
    let path = builder.add_text_field("path", STRING | STORED | FAST);
    let text = builder.add_text_field("text", text_opts);
    let xattrs_json = builder.add_text_field("xattrs_json", STORED);
    let model_version = builder.add_text_field("model_version", STRING | STORED);

    let schema = builder.build();
    let fields = IndexSchema {
        hash_hex,
        path,
        text,
        xattrs_json,
        model_version,
    };
    (schema, fields)
}

impl TextIndex {
    pub fn open(dir: impl AsRef<Path>) -> Result<Self> {
        let dir = dir.as_ref();
        std::fs::create_dir_all(dir)?;

        let (schema, fields) = build_schema();

        let index = if dir.join("meta.json").exists() {
            Index::open_in_dir(dir)?
        } else {
            Index::create_in_dir(dir, schema)?
        };

        index
            .tokenizers()
            .register("en_stem", tantivy::tokenizer::SimpleTokenizer::default());

        let writer = index.writer(50_000_000)?;
        Ok(Self {
            index,
            writer,
            schema: fields,
        })
    }

    pub fn index(&mut self, doc: &Document) -> Result<()> {
        // Delete any existing entry for this hash so re-indexing is idempotent.
        self.delete(&doc.file_hash)?;

        let xattrs_json = serde_json::to_string(&doc.xattrs)?;
        let mut tdoc = TantivyDocument::default();
        tdoc.add_text(self.schema.hash_hex, doc.file_hash.to_hex());
        tdoc.add_text(self.schema.path, &doc.path);
        tdoc.add_text(self.schema.text, &doc.text);
        tdoc.add_text(self.schema.xattrs_json, &xattrs_json);
        tdoc.add_text(self.schema.model_version, &doc.model_version);
        self.writer.add_document(tdoc)?;
        self.writer.commit()?;
        Ok(())
    }

    pub fn delete(&mut self, hash: &Hash) -> Result<()> {
        use tantivy::query::TermQuery;
        use tantivy::schema::IndexRecordOption;
        use tantivy::Term;

        let term = Term::from_field_text(self.schema.hash_hex, &hash.to_hex());
        let q = TermQuery::new(term, IndexRecordOption::Basic);
        self.writer.delete_query(Box::new(q))?;
        self.writer.commit()?;
        Ok(())
    }

    pub fn search(&self, query_str: &str, limit: usize) -> Result<Vec<SearchResult>> {
        let reader = self
            .index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()?;
        let searcher = reader.searcher();

        let parser = QueryParser::for_index(&self.index, vec![self.schema.text, self.schema.path]);
        let query = parser
            .parse_query(query_str)
            .map_err(|e| IndexError::Query(e.to_string()))?;

        let top_docs = searcher.search(&query, &TopDocs::with_limit(limit))?;

        let mut results = Vec::with_capacity(top_docs.len());
        for (score, addr) in top_docs {
            let retrieved: TantivyDocument = searcher.doc(addr)?;
            let hash_hex = retrieved
                .get_first(self.schema.hash_hex)
                .and_then(|v| {
                    if let tantivy::schema::OwnedValue::Str(s) = v {
                        Some(s.as_str())
                    } else {
                        None
                    }
                })
                .unwrap_or_default();
            let path = retrieved
                .get_first(self.schema.path)
                .and_then(|v| {
                    if let tantivy::schema::OwnedValue::Str(s) = v {
                        Some(s.as_str())
                    } else {
                        None
                    }
                })
                .unwrap_or_default()
                .to_string();
            let xattrs_json = retrieved
                .get_first(self.schema.xattrs_json)
                .and_then(|v| {
                    if let tantivy::schema::OwnedValue::Str(s) = v {
                        Some(s.as_str())
                    } else {
                        None
                    }
                })
                .unwrap_or("{}");
            let xattrs: HashMap<String, String> =
                serde_json::from_str(xattrs_json).unwrap_or_default();

            let file_hash = Hash::from_hex(hash_hex).unwrap_or(Hash::ZERO);
            results.push(SearchResult {
                file_hash,
                path,
                score,
                snippet: None,
                xattrs,
            });
        }
        Ok(results)
    }
}
