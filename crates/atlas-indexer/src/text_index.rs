//! Tantivy-backed full-text index.

use crate::{Document, IndexError, Result, SearchResult};
use atlas_core::Hash;
use std::collections::HashMap;
use std::path::Path;
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::{Field, Schema, TextFieldIndexing, TextOptions, FAST, STORED, STRING};
use tantivy::{Index, IndexWriter, ReloadPolicy, TantivyDocument};

/// Extract the first string value of a stored field.
fn str_field<'a>(doc: &'a TantivyDocument, field: Field) -> Option<&'a str> {
    doc.get_first(field).and_then(|v| {
        if let tantivy::schema::OwnedValue::Str(s) = v {
            Some(s.as_str())
        } else {
            None
        }
    })
}

/// Strip simple HTML tags (e.g. `<b>`, `</b>`) from a snippet string.
fn strip_html_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

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
        use tantivy::snippet::SnippetGenerator;

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

        // Build a snippet generator once per search — it analyses the query
        // against the `text` field and extracts the best matching excerpt.
        let snippet_gen =
            SnippetGenerator::create(&searcher, &*query, self.schema.text).ok();

        let mut results = Vec::with_capacity(top_docs.len());
        for (score, addr) in top_docs {
            let retrieved: TantivyDocument = searcher.doc(addr)?;

            let hash_hex = str_field(&retrieved, self.schema.hash_hex).unwrap_or_default();
            let path = str_field(&retrieved, self.schema.path)
                .unwrap_or_default()
                .to_string();
            let xattrs_json =
                str_field(&retrieved, self.schema.xattrs_json).unwrap_or("{}");
            let xattrs: HashMap<String, String> =
                serde_json::from_str(xattrs_json).unwrap_or_default();

            // Generate a context-rich snippet with matched terms highlighted.
            let snippet = snippet_gen.as_ref().map(|gen| {
                let s = gen.snippet_from_doc(&retrieved);
                // `to_html()` wraps matched terms in <b>…</b>; strip the tags
                // so callers get plain text they can display however they want.
                strip_html_tags(&s.to_html())
            });

            let file_hash = Hash::from_hex(hash_hex).unwrap_or(Hash::ZERO);
            results.push(SearchResult {
                file_hash,
                path,
                score,
                snippet,
                xattrs,
            });
        }
        Ok(results)
    }
}
