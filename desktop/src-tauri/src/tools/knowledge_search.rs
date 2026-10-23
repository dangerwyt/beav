use glob::{MatchOptions, Pattern};
use rusqlite::params;
use serde_json::{json, Value};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;
use tauri::State;

use crate::knowledge_index::{
    advisor_source_id, citation_anchors, document_blocks,
    hybrid::RetrievalMode,
    index_status, open_catalog_connection,
    query_profile::{self, QueryProfile},
    retrieval_audit,
};
use crate::persistence::with_store;
use crate::{payload_field, payload_string, AppState};
use crate::DocumentKnowledgeSourceRecord;

const DEFAULT_GLOB_LIMIT: usize = 50;
const MAX_GLOB_LIMIT: usize = 200;
const DEFAULT_GREP_LIMIT: usize = 20;
const MAX_GREP_LIMIT: usize = 100;
const DEFAULT_READ_LIMIT: usize = 160;
const MAX_READ_LIMIT: usize = 400;
const DEFAULT_READ_MAX_CHARS: usize = 8000;
const DEFAULT_SNIPPET_CHARS: usize = 220;
const DEFAULT_ATTACH_MAX_BYTES: u64 = 20 * 1024 * 1024;

#[derive(Debug, Clone)]
enum KnowledgeScopeKind {
    Advisor,
    DocumentSource,
    Workspace,
}

#[derive(Debug, Clone)]
struct KnowledgeScope {
    kind: KnowledgeScopeKind,
    member_id: Option<String>,
    member_name: Option<String>,
    advisor_id: Option<String>,
    advisor_name: Option<String>,
    language: Option<String>,
    language_detection_status: Option<String>,
    source_id: Option<String>,
    source_name: Option<String>,
    root: PathBuf,
}

pub fn execute_glob(
    state: &State<'_, AppState>,
    session_id: Option<&str>,
    arguments: &Value,
) -> Result<Value, String> {
    let scope = resolve_scope(state, session_id, arguments)?;
    let limit = parse_usize(arguments, "limit", DEFAULT_GLOB_LIMIT, MAX_GLOB_LIMIT);
    // [KB-PATH-FIX] A canonical doc-source path may resolve to the source's real rootPath.
    let (pattern_text, root_override) = list_pattern_for_scope_with_remap(state, &scope, arguments)?;
    let pattern = compile_pattern(&pattern_text)?;
    let glob_root = root_override.unwrap_or_else(|| scope.root.clone());
    if let Some(source_id) = scope.source_id.as_deref() {
        let indexed_count = document_blocks::count_blocks_for_source(state, source_id)?;
        if indexed_count > 0 {
            let documents =
                document_blocks::list_documents_for_source(state, source_id, &pattern, limit)?;
            let total_matches = documents.len();
            return Ok(json!({
                "scopeKind": scope_kind_label(&scope),
                "memberId": scope.member_id,
                "memberName": scope.member_name,
                "advisorId": scope.advisor_id,
                "advisorName": scope.advisor_name,
                "language": scope.language,
                "languageDetectionStatus": scope.language_detection_status,
                "sourceId": scope.source_id,
                "sourceName": scope.source_name,
                "rootPath": glob_root.display().to_string(),
                "pattern": pattern_text,
                "searchMode": "indexed-documents",
                "totalMatches": total_matches,
                "files": documents.into_iter().map(|item| {
                    json!({
                        "path": item.path,
                        "name": item.name,
                        "title": item.title,
                        "language": item.language,
                        "extension": item.extension,
                        "sizeBytes": item.size_bytes,
                        "updatedAt": item.updated_at
                    })
                }).collect::<Vec<_>>()
            }));
        }
    }
    if matches!(scope.kind, KnowledgeScopeKind::DocumentSource) {
        let mut matched = collect_matching_files(&glob_root, &pattern)?;
        matched.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        let total_matches = matched.len();
        matched.truncate(limit);
        return Ok(json!({
            "scopeKind": scope_kind_label(&scope),
            "memberId": scope.member_id,
            "memberName": scope.member_name,
            "advisorId": scope.advisor_id,
            "advisorName": scope.advisor_name,
            "language": scope.language,
            "languageDetectionStatus": scope.language_detection_status,
            "sourceId": scope.source_id,
            "sourceName": scope.source_name,
            "rootPath": scoped_root_path(&scope),
            "pattern": pattern_text,
            "searchMode": "filesystem-fallback",
            "totalMatches": total_matches,
            "files": matched.into_iter().map(|item| {
                json!({
                    "path": item.relative_path,
                    "name": item.name,
                    "extension": item.extension,
                    "sizeBytes": item.size_bytes,
                    "updatedAt": item.updated_at_ms
                })
            }).collect::<Vec<_>>()
        }));
    }
    let mut matched = collect_matching_files(&glob_root, &pattern)?;
    matched.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    let total_matches = matched.len();
    matched.truncate(limit);

    Ok(json!({
        "scopeKind": scope_kind_label(&scope),
        "memberId": scope.member_id,
        "memberName": scope.member_name,
        "advisorId": scope.advisor_id,
        "advisorName": scope.advisor_name,
        "language": scope.language,
        "languageDetectionStatus": scope.language_detection_status,
        "sourceId": scope.source_id,
        "sourceName": scope.source_name,
        "rootPath": glob_root.display().to_string(),
        "pattern": pattern_text,
        "searchMode": "filesystem-fallback",
        "totalMatches": total_matches,
        "files": matched.into_iter().map(|item| {
            json!({
                "path": item.relative_path,
                "name": item.name,
                "extension": item.extension,
                "sizeBytes": item.size_bytes,
                "updatedAt": item.updated_at_ms
            })
        }).collect::<Vec<_>>()
    }))
}

pub fn execute_grep(
    state: &State<'_, AppState>,
    session_id: Option<&str>,
    arguments: &Value,
) -> Result<Value, String> {
    let scope = resolve_scope(state, session_id, arguments)?;
    let query = payload_string(arguments, "query")
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "Search(path=\"knowledge://\", query=\"...\") requires query".to_string())?;
    if scope.source_id.is_some() {
        return execute_source_search(state, &scope, arguments, &query);
    }
    // [KB-PATH-FIX] A canonical doc-source path may resolve to the source's real rootPath.
    let (pattern_text, root_override) = search_pattern_for_scope_with_remap(state, &scope, arguments)?;
    let pattern = compile_pattern(&pattern_text)?;
    let grep_root = root_override.unwrap_or_else(|| scope.root.clone());
    let limit = parse_usize(arguments, "limit", DEFAULT_GREP_LIMIT, MAX_GREP_LIMIT);
    let snippet_chars = parse_usize(arguments, "snippetChars", DEFAULT_SNIPPET_CHARS, 800);
    let query_lower = query.to_lowercase();
    let mut files = collect_matching_files(&grep_root, &pattern)?;
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));

    let mut hits = Vec::<Value>::new();
    for file in files {
        if hits.len() >= limit {
            break;
        }
        if !is_text_file(&file.absolute_path) {
            continue;
        }
        let Ok(content) = fs::read_to_string(&file.absolute_path) else {
            continue;
        };
        for (index, line) in content.lines().enumerate() {
            if !line.to_lowercase().contains(&query_lower) {
                continue;
            }
            hits.push(json!({
                "documentId": format!("{}:{}", scope.advisor_id.clone().unwrap_or_default(), file.relative_path),
                "blockId": Value::Null,
                "path": file.relative_path,
                "name": file.name,
                "blockType": Value::Null,
                "sectionPath": Value::Null,
                "page": Value::Null,
                "lineNumber": index + 1,
                "legalMetadata": Value::Null,
                "snippet": truncate_chars(line.trim(), snippet_chars),
            }));
            if hits.len() >= limit {
                break;
            }
        }
    }

    Ok(json!({
        "scopeKind": scope_kind_label(&scope),
        "memberId": scope.member_id,
        "memberName": scope.member_name,
        "advisorId": scope.advisor_id,
        "advisorName": scope.advisor_name,
        "language": scope.language,
        "languageDetectionStatus": scope.language_detection_status,
        "sourceId": scope.source_id,
        "sourceName": scope.source_name,
        "rootPath": grep_root.display().to_string(),
        "pattern": pattern_text,
        "query": query,
        "totalMatches": hits.len(),
        "hits": hits
    }))
}

// [KB-PATH-FIX] Non-copied knowledge sources (tracked-folder / obsidian-vault) register an
// existing directory and are NOT mirrored into `knowledge/docs/doc-source-<id>/`. When the
// authoring runtime reads the canonical `docs/doc-source-<id>/{meta.json,content.md}` path,
// that file does not exist for those sources, so the read fails. This resolves the canonical
// path back to the source's real `rootPath` so tracked/obsidian material can actually be read.
#[derive(Debug, Clone)]
enum KbResolved {
    Text(String),
    File(PathBuf),
}

// [KB-PATH-FIX] Split a canonical knowledge path into (source_id, rest-relative-to-source-root).
// Accepts `docs/doc-source-<id>/...`, `doc-source-<id>/...`, and bare `content.md`-style paths
// (the latter resolved through `source_id_hint`, i.e. a source-scoped call).
fn parse_kb_source_path(
    relative_path: &str,
    source_id_hint: Option<&str>,
) -> Option<(String, String)> {
    let trimmed = relative_path
        .trim_start_matches("knowledge/")
        .trim_start_matches("knowledge\\")
        .trim_start_matches('/');
    let parts: Vec<&str> = trimmed.splitn(3, '/').collect();
    let (source_id, rest) = match parts.as_slice() {
        ["docs", sid, rest] => (*sid, *rest),
        ["docs", sid] => (*sid, ""),
        [sid, rest, ..] if sid.starts_with("doc-source-") => (*sid, *rest),
        [sid] if sid.starts_with("doc-source-") => (*sid, ""),
        // [KB-PATH-FIX] No doc-source prefix in the path (e.g. a source-scoped read of
        // "content.md"/"meta.json"); fall back to the scope's source id if present.
        _ => (source_id_hint.unwrap_or(""), ""),
    };
    let source_id = source_id.trim();
    if source_id.is_empty() {
        return None;
    }
    Some((source_id.to_string(), rest.to_string()))
}

// [KB-PATH-FIX] Snapshot the registered document sources. Kept separate from the pure
// resolution helpers below so the resolution logic can be exercised in unit tests against real
// `sources.json` data without a live Tauri `State`.
fn document_sources_snapshot(
    state: &State<'_, AppState>,
) -> Result<Vec<DocumentKnowledgeSourceRecord>, String> {
    with_store(state, |store| Ok(store.document_sources.clone()))
}

// [KB-PATH-FIX] The store keeps `document_sources[].id` WITH the `doc-source-` prefix
// (e.g. "doc-source-1792121765191"), while request paths may carry either that form or the
// bare id. Compare on the bare id so both resolve — stripping the prefix and comparing against
// the stored id never matches, which silently disabled the whole remap.
fn find_document_source<'a>(
    sources: &'a [DocumentKnowledgeSourceRecord],
    source_id: &str,
) -> Option<&'a DocumentKnowledgeSourceRecord> {
    let bare_id = source_id.trim_start_matches("doc-source-");
    sources.iter().find(|item| {
        item.id == source_id
            || item.id == bare_id
            || item.id.trim_start_matches("doc-source-") == bare_id
    })
}

// [KB-PATH-FIX] Resolve a canonical doc-source path to (real source root, remainder relative to
// that root). Used by list/search so they can enumerate the real files of non-mirrored sources
// instead of failing on the missing `knowledge/docs/doc-source-<id>/` directory.
fn resolve_kb_source_root_from_sources(
    sources: &[DocumentKnowledgeSourceRecord],
    source_id_hint: Option<&str>,
    relative_path: &str,
) -> Option<(PathBuf, String)> {
    let (source_id, rest) = parse_kb_source_path(relative_path, source_id_hint)?;
    let source = find_document_source(sources, &source_id)?;
    // copied-file sources ARE mirrored, so their canonical path is already correct.
    if source.kind == "copied-file" {
        return None;
    }
    let root = PathBuf::from(&source.root_path);
    if !root.exists() {
        return None;
    }
    Some((root, rest))
}

fn resolve_kb_source_root(
    state: &State<'_, AppState>,
    source_id_hint: Option<&str>,
    relative_path: &str,
) -> Result<Option<(PathBuf, String)>, String> {
    let sources = document_sources_snapshot(state)?;
    Ok(resolve_kb_source_root_from_sources(
        &sources,
        source_id_hint,
        relative_path,
    ))
}

fn resolve_kb_source_canonical_from_sources(
    sources: &[DocumentKnowledgeSourceRecord],
    source_id_hint: Option<&str>,
    relative_path: &str,
) -> Result<Option<KbResolved>, String> {
    let Some((source_id, rest)) = parse_kb_source_path(relative_path, source_id_hint) else {
        return Ok(None);
    };
    let Some(source) = find_document_source(sources, &source_id) else {
        return Ok(None);
    };
    // copied-file sources ARE mirrored into knowledge/docs/doc-source-<id>, so the canonical
    // path is already correct for them; only remap non-copied kinds.
    if source.kind == "copied-file" {
        return Ok(None);
    }
    let root = PathBuf::from(&source.root_path);
    if !root.exists() {
        return Ok(None);
    }
    if rest.is_empty() || rest == "content.md" || rest == "content.txt" || rest == "index.md" {
        // tracked/obsidian sources have no single mirrored content.md; aggregate the real
        // text files under rootPath into one virtual document.
        return Ok(Some(KbResolved::Text(aggregate_source_text_files(&root)?)));
    }
    if rest == "meta.json" {
        return Ok(Some(KbResolved::Text(
            serde_json::to_string_pretty(&synthesize_source_meta(source)).unwrap_or_default(),
        )));
    }
    let candidate = root.join(&rest);
    if candidate.exists() && candidate.is_file() {
        return Ok(Some(KbResolved::File(candidate)));
    }
    Ok(None)
}

fn resolve_kb_source_canonical(
    state: &State<'_, AppState>,
    source_id_hint: Option<&str>,
    relative_path: &str,
) -> Result<Option<KbResolved>, String> {
    let sources = document_sources_snapshot(state)?;
    resolve_kb_source_canonical_from_sources(&sources, source_id_hint, relative_path)
}

fn aggregate_source_text_files(root: &Path) -> Result<String, String> {
    let pattern = compile_pattern("**/*")?;
    let mut matched = collect_matching_files(root, &pattern)?;
    matched.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    let mut out = String::new();
    for file in matched {
        if !is_text_file(&file.absolute_path) {
            continue;
        }
        match fs::read_to_string(&file.absolute_path) {
            Ok(text) => {
                out.push_str(&format!("\n\n// === {} ===\n", file.relative_path));
                out.push_str(&text);
            }
            Err(_) => continue,
        }
    }
    if out.trim().is_empty() {
        out.push_str("(source contains no readable text files)");
    }
    Ok(out)
}

fn synthesize_source_meta(source: &DocumentKnowledgeSourceRecord) -> Value {
    json!({
        "id": source.id,
        "name": source.name,
        "kind": source.kind,
        "rootPath": source.root_path,
        "fileCount": source.file_count,
        "sampleFiles": source.sample_files,
        "createdAt": source.created_at,
        "updatedAt": source.updated_at,
        "note": "synthesized by KB-PATH-FIX for a non-copied source (tracked-folder / obsidian-vault); original files live at rootPath and are not mirrored into knowledge/docs/doc-source-<id>",
    })
}

fn respond_kb_read(
    scope: &KnowledgeScope,
    relative_path: &str,
    absolute_path_display: &str,
    content: String,
    max_chars: usize,
    offset: usize,
    limit: usize,
) -> Value {
    let lines: Vec<&str> = content.lines().collect();
    let safe_offset = offset.min(lines.len());
    let line_end = safe_offset.saturating_add(limit).min(lines.len());
    let sliced = lines[safe_offset..line_end].join("\n");
    let truncated = sliced.chars().count() > max_chars;
    json!({
        "scopeKind": scope_kind_label(scope),
        "memberId": scope.member_id,
        "memberName": scope.member_name,
        "advisorId": scope.advisor_id,
        "advisorName": scope.advisor_name,
        "language": scope.language,
        "languageDetectionStatus": scope.language_detection_status,
        "sourceId": scope.source_id,
        "sourceName": scope.source_name,
        "rootPath": scope.root.display().to_string(),
        "path": relative_path,
        "absolutePath": absolute_path_display,
        "lineStart": if line_end > safe_offset { safe_offset + 1 } else { 0 },
        "lineEnd": line_end,
        "totalLines": lines.len(),
        "truncated": truncated,
        "content": truncate_chars(&sliced, max_chars)
    })
}

pub fn execute_read(
    state: &State<'_, AppState>,
    session_id: Option<&str>,
    arguments: &Value,
) -> Result<Value, String> {
    let scope = resolve_scope(state, session_id, arguments)?;
    if scope.source_id.is_some() {
        if let Some(anchor_id) =
            payload_string(arguments, "anchorId").filter(|value| !value.trim().is_empty())
        {
            if let Some(anchor) = citation_anchors::read_anchor(state, &anchor_id)? {
                let block = document_blocks::read_block(state, &anchor.block_id)?;
                let visual_evidence = match block.as_ref() {
                    Some(item) => visual_evidence_for_block(
                        state,
                        item.visual_unit_id.as_deref(),
                        &item.evidence_refs_json,
                    )?,
                    None => Vec::new(),
                };
                return Ok(json!({
                    "scopeKind": scope_kind_label(&scope),
                    "memberId": scope.member_id,
                    "memberName": scope.member_name,
                    "scopeLanguage": scope.language,
                    "languageDetectionStatus": scope.language_detection_status,
                    "sourceId": anchor.source_id,
                    "sourceName": anchor.source_name,
                    "documentId": anchor.document_id,
                    "blockId": anchor.block_id,
                    "anchorId": anchor.anchor_id,
                    "rootPath": anchor.root_path,
                    "path": anchor.path,
                    "absolutePath": anchor.absolute_path,
                    "title": anchor.title,
                    "language": anchor.language,
                    "page": anchor.page,
                    "blockType": anchor.block_type,
                    "sectionPath": anchor.section_path,
                    "charStart": anchor.char_start,
                    "charEnd": anchor.char_end,
                    "lineStart": anchor.line_start,
                    "lineEnd": anchor.line_end,
                    "contentOrigin": block.as_ref().map(|item| item.content_origin.clone()),
                    "ocrConfidence": block.as_ref().and_then(|item| legacy_ocr_confidence(&item.content_origin, item.ocr_confidence)),
                    "visualSource": block.as_ref().and_then(|item| visual_source_for_block(
                        &item.content_origin,
                        &item.source_id,
                        &item.document_id,
                        &item.relative_path,
                        &item.absolute_path,
                        item.page,
                        &item.block_type,
                        item.visual_unit_id.as_deref(),
                        item.source_document_id.as_deref(),
                        &item.evidence_refs_json
                    )),
                    "visualEvidence": visual_evidence,
                    "content": truncate_chars(&anchor.quote_text, parse_usize(arguments, "maxChars", DEFAULT_READ_MAX_CHARS, 20_000))
                }));
            }
            return Err(format!("knowledge anchor does not exist: {anchor_id}"));
        }
        if let Some(block_id) =
            payload_string(arguments, "blockId").filter(|value| !value.trim().is_empty())
        {
            if let Some(block) = document_blocks::read_block(state, &block_id)? {
                let anchors = citation_anchors::anchors_for_block(state, &block.block_id)?;
                let visual_evidence = visual_evidence_for_block(
                    state,
                    block.visual_unit_id.as_deref(),
                    &block.evidence_refs_json,
                )?;
                return Ok(json!({
                    "scopeKind": scope_kind_label(&scope),
                    "memberId": scope.member_id,
                    "memberName": scope.member_name,
                    "scopeLanguage": scope.language,
                    "languageDetectionStatus": scope.language_detection_status,
                    "sourceId": block.source_id,
                    "sourceName": block.source_name,
                    "documentId": block.document_id,
                    "blockId": block.block_id,
                    "anchorIds": anchors.iter().map(|item| item.anchor_id.clone()).collect::<Vec<_>>(),
                    "anchors": anchors,
                    "rootPath": block.root_path,
                    "path": block.relative_path,
                    "absolutePath": block.absolute_path,
                    "title": block.title,
                    "language": block.language,
                    "contentOrigin": block.content_origin,
                    "ocrConfidence": legacy_ocr_confidence(&block.content_origin, block.ocr_confidence),
                    "visualSource": visual_source_for_block(
                        &block.content_origin,
                        &block.source_id,
                        &block.document_id,
                        &block.relative_path,
                        &block.absolute_path,
                        block.page,
                        &block.block_type,
                        block.visual_unit_id.as_deref(),
                        block.source_document_id.as_deref(),
                        &block.evidence_refs_json
                    ),
                    "visualEvidence": visual_evidence,
                    "legalMetadata": {
                        "jurisdiction": block.jurisdiction,
                        "authority": block.authority,
                        "authorityLevel": block.authority_level,
                        "effectiveDate": block.effective_date,
                        "expiryDate": block.expiry_date,
                        "documentType": block.document_type,
                        "isSuperseded": block.is_superseded
                    },
                    "page": block.page,
                    "blockType": block.block_type,
                    "sectionPath": serde_json::from_str::<Vec<String>>(&block.section_path_json).unwrap_or_default(),
                    "blockIndex": block.block_index,
                    "lineStart": block.line_start,
                    "lineEnd": block.line_end,
                    "content": truncate_chars(&block.text, parse_usize(arguments, "maxChars", DEFAULT_READ_MAX_CHARS, 20_000))
                }));
            }
            return Err(format!("knowledge block does not exist: {block_id}"));
        }
    }
    let relative_path = payload_string(arguments, "path")
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "Read(path=\"knowledge://...\") requires path".to_string())
        .and_then(|value| normalize_scope_relative_path(&scope, &value))?;
    let offset = parse_usize(arguments, "offset", 0, usize::MAX);
    let limit = parse_usize(arguments, "limit", DEFAULT_READ_LIMIT, MAX_READ_LIMIT);
    let max_chars = parse_usize(arguments, "maxChars", DEFAULT_READ_MAX_CHARS, 20_000);
    let target_path = resolve_relative_path(&scope.root, &relative_path)?;
    if !target_path.exists() {
        // [KB-PATH-FIX] Non-copied sources (tracked-folder / obsidian-vault) are NOT mirrored
        // into knowledge/docs/doc-source-<id>/. The authoring runtime reads the canonical
        // `docs/doc-source-<id>/{meta.json,content.md}` path, which does not exist for those
        // sources, so the read fails. Remap the canonical path back to the source's real
        // rootPath so the registered material becomes readable. copied-file sources are left
        // untouched (their canonical path is correct), and genuinely missing files still error.
        if let Some(resolved) =
            resolve_kb_source_canonical(state, scope.source_id.as_deref(), &relative_path)?
        {
            return Ok(match resolved {
                KbResolved::Text(text) => respond_kb_read(
                    &scope,
                    &relative_path,
                    "<remapped:source-rootPath>",
                    text,
                    max_chars,
                    offset,
                    limit,
                ),
                KbResolved::File(path) => {
                    let content = fs::read_to_string(&path).map_err(|error| error.to_string())?;
                    respond_kb_read(
                        &scope,
                        &relative_path,
                        &path.display().to_string(),
                        content,
                        max_chars,
                        offset,
                        limit,
                    )
                }
            });
        }
        return Err(format!("knowledge file does not exist: {relative_path}"));
    }
    if !target_path.is_file() {
        return Err(format!("knowledge path is not a file: {relative_path}"));
    }
    let content = fs::read_to_string(&target_path).map_err(|error| error.to_string())?;
    let lines = content.lines().collect::<Vec<_>>();
    let safe_offset = offset.min(lines.len());
    let line_end = safe_offset.saturating_add(limit).min(lines.len());
    let sliced = lines[safe_offset..line_end].join("\n");
    let truncated = sliced.chars().count() > max_chars;

    Ok(json!({
        "scopeKind": scope_kind_label(&scope),
        "memberId": scope.member_id,
        "memberName": scope.member_name,
        "advisorId": scope.advisor_id,
        "advisorName": scope.advisor_name,
        "language": scope.language,
        "languageDetectionStatus": scope.language_detection_status,
        "sourceId": scope.source_id,
        "sourceName": scope.source_name,
        "rootPath": scope.root.display().to_string(),
        "path": relative_path,
        "absolutePath": target_path.display().to_string(),
        "lineStart": if line_end > safe_offset { safe_offset + 1 } else { 0 },
        "lineEnd": line_end,
        "totalLines": lines.len(),
        "truncated": truncated,
        "content": truncate_chars(&sliced, max_chars)
    }))
}

pub fn execute_attach(
    state: &State<'_, AppState>,
    session_id: Option<&str>,
    arguments: &Value,
) -> Result<Value, String> {
    let scope = resolve_scope(state, session_id, arguments)?;
    let relative_path = payload_string(arguments, "path")
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            "Operate(resource=\"knowledge\", operation=\"attach\", input={...}) requires path"
                .to_string()
        })
        .and_then(|value| normalize_scope_relative_path(&scope, &value))?;
    let target_path = resolve_relative_path(&scope.root, &relative_path)?;
    if !target_path.exists() {
        return Err(format!("knowledge file does not exist: {relative_path}"));
    }
    if !target_path.is_file() {
        return Err(format!("knowledge path is not a file: {relative_path}"));
    }
    let metadata = fs::metadata(&target_path).map_err(|error| error.to_string())?;
    if metadata.len() == 0 {
        return Err(format!("knowledge attachment is empty: {relative_path}"));
    }
    let max_bytes = payload_field(arguments, "maxBytes")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_ATTACH_MAX_BYTES)
        .min(DEFAULT_ATTACH_MAX_BYTES);
    if metadata.len() > max_bytes {
        return Err(format!(
            "knowledge attachment is too large for direct model input: {} bytes > {} bytes",
            metadata.len(),
            max_bytes
        ));
    }
    let (mime_type, kind, _) = crate::guess_mime_and_kind(&target_path);
    if !matches!(kind.as_str(), "image" | "audio" | "video") {
        return Err(format!(
            "knowledge.attach only supports image/audio/video files, got {kind}: {relative_path}"
        ));
    }
    let name = target_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("knowledge-attachment")
        .to_string();
    let absolute_path = target_path.display().to_string();
    Ok(json!({
        "scopeKind": scope_kind_label(&scope),
        "memberId": scope.member_id,
        "memberName": scope.member_name,
        "advisorId": scope.advisor_id,
        "advisorName": scope.advisor_name,
        "language": scope.language,
        "languageDetectionStatus": scope.language_detection_status,
        "sourceId": scope.source_id,
        "sourceName": scope.source_name,
        "rootPath": scope.root.display().to_string(),
        "path": relative_path,
        "absolutePath": absolute_path,
        "name": name,
        "kind": kind,
        "mimeType": mime_type,
        "sizeBytes": metadata.len(),
        "llmInputAttachments": [{
            "type": "uploaded-file",
            "name": name,
            "kind": kind,
            "mimeType": mime_type,
            "size": metadata.len(),
            "absolutePath": absolute_path,
            "originalAbsolutePath": absolute_path,
            "workspaceRelativePath": format!("knowledge/{relative_path}"),
            "path": relative_path,
            "deliveryMode": "direct-input",
            "requiresMultimodal": true,
            "source": "knowledge"
        }]
    }))
}

#[derive(Debug, Clone)]
struct MatchedFile {
    absolute_path: PathBuf,
    relative_path: String,
    name: String,
    extension: Option<String>,
    size_bytes: u64,
    updated_at_ms: i64,
}

fn resolve_scope(
    state: &State<'_, AppState>,
    session_id: Option<&str>,
    arguments: &Value,
) -> Result<KnowledgeScope, String> {
    if let Some(advisor_id) = payload_string(arguments, "advisorId") {
        return advisor_scope(state, &advisor_id, None, None);
    }

    if let Some(member_scope) = resolve_member_scope(state, session_id, arguments)? {
        return Ok(member_scope);
    }

    if let Some(advisor_id) = resolve_session_advisor_id(state, session_id) {
        return advisor_scope(state, &advisor_id, None, None);
    }

    if let Some(source_scope) = resolve_document_source_scope(state, arguments)? {
        return Ok(source_scope);
    }

    Ok(KnowledgeScope {
        kind: KnowledgeScopeKind::Workspace,
        member_id: None,
        member_name: None,
        advisor_id: None,
        advisor_name: None,
        language: None,
        language_detection_status: None,
        source_id: None,
        source_name: None,
        root: crate::workspace_root(state)?.join("knowledge"),
    })
}

fn advisor_scope(
    state: &State<'_, AppState>,
    advisor_id: &str,
    member_id: Option<String>,
    member_name: Option<String>,
) -> Result<KnowledgeScope, String> {
    let advisor = with_store(state, |store| {
        Ok(store
            .advisors
            .iter()
            .find(|item| item.id == advisor_id)
            .map(|item| {
                (
                    item.id.clone(),
                    item.name.clone(),
                    item.detected_knowledge_language
                        .clone()
                        .or(item.knowledge_language.clone()),
                    item.language_detection_status.clone(),
                )
            }))
    })?
    .ok_or_else(|| format!("advisor not found: {advisor_id}"))?;
    let root = crate::advisor_knowledge_dir(state, &advisor.0)?;
    Ok(KnowledgeScope {
        kind: KnowledgeScopeKind::Advisor,
        member_id,
        member_name,
        advisor_id: Some(advisor.0),
        advisor_name: Some(advisor.1.clone()),
        language: advisor.2,
        language_detection_status: advisor.3,
        source_id: Some(advisor_source_id(advisor_id)),
        source_name: Some(advisor.1),
        root,
    })
}

fn resolve_member_scope(
    state: &State<'_, AppState>,
    session_id: Option<&str>,
    arguments: &Value,
) -> Result<Option<KnowledgeScope>, String> {
    let requested_member_id = payload_string(arguments, "memberId")
        .or_else(|| payload_string(arguments, "collabMemberId"))
        .or_else(|| resolve_session_collab_member_id(state, session_id));
    let Some(member_id) = requested_member_id else {
        return Ok(None);
    };
    let member = with_store(state, |store| {
        Ok(store
            .collab_members
            .iter()
            .find(|item| item.id == member_id)
            .cloned())
    })?
    .ok_or_else(|| format!("collaboration member not found: {member_id}"))?;
    let metadata = member.metadata.as_ref();
    if let Some(advisor_id) = metadata.and_then(|value| payload_string(value, "advisorId")) {
        return advisor_scope(
            state,
            &advisor_id,
            Some(member.id.clone()),
            Some(member.display_name.clone()),
        )
        .map(Some);
    }
    if let Some(source_id) = metadata
        .and_then(|value| payload_string(value, "sourceId"))
        .or_else(|| metadata.and_then(|value| payload_string(value, "knowledgeSourceId")))
    {
        if let Some(advisor_id) = source_id.strip_prefix("advisor:") {
            return advisor_scope(
                state,
                advisor_id,
                Some(member.id.clone()),
                Some(member.display_name.clone()),
            )
            .map(Some);
        }
        let args = json!({ "sourceId": source_id });
        if let Some(mut scope) = resolve_document_source_scope(state, &args)? {
            scope.member_id = Some(member.id.clone());
            scope.member_name = Some(member.display_name.clone());
            return Ok(Some(scope));
        }
    }
    if let Some(root_path) = metadata
        .and_then(|value| payload_string(value, "rootPath"))
        .or_else(|| metadata.and_then(|value| payload_string(value, "knowledgeRootPath")))
    {
        let args = json!({ "rootPath": root_path });
        if let Some(mut scope) = resolve_document_source_scope(state, &args)? {
            scope.member_id = Some(member.id.clone());
            scope.member_name = Some(member.display_name.clone());
            return Ok(Some(scope));
        }
    }
    let matched_advisor_id = with_store(state, |store| {
        Ok(store
            .advisors
            .iter()
            .find(|advisor| {
                advisor.id == member.role_id
                    || advisor.id == member.display_name
                    || advisor.name == member.display_name
            })
            .map(|advisor| advisor.id.clone()))
    })?;
    if let Some(advisor_id) = matched_advisor_id {
        return advisor_scope(
            state,
            &advisor_id,
            Some(member.id),
            Some(member.display_name),
        )
        .map(Some);
    }
    Err(format!(
        "collaboration member {} is not bound to a knowledge source; set metadata.advisorId or metadata.sourceId",
        member.id
    ))
}

fn resolve_document_source_scope(
    state: &State<'_, AppState>,
    arguments: &Value,
) -> Result<Option<KnowledgeScope>, String> {
    let requested_source_id =
        payload_string(arguments, "sourceId").filter(|value| !value.trim().is_empty());
    let requested_root_path =
        payload_string(arguments, "rootPath").filter(|value| !value.trim().is_empty());
    if requested_source_id.is_none() && requested_root_path.is_none() {
        return Ok(None);
    }
    let requested_root_path = requested_root_path.as_deref().map(PathBuf::from);
    let matched = with_store(state, |store| {
        Ok(store
            .document_sources
            .iter()
            .find(|item| {
                requested_source_id.as_deref() == Some(item.id.as_str())
                    || requested_root_path
                        .as_ref()
                        .is_some_and(|path| path == &PathBuf::from(&item.root_path))
            })
            .map(|item| (item.id.clone(), item.name.clone(), item.root_path.clone())))
    })?;
    let Some((source_id, source_name, root_path)) = matched else {
        return Err("registered document source not found".to_string());
    };
    let root = PathBuf::from(root_path);
    if !root.exists() {
        return Err(format!(
            "document source root does not exist: {}",
            root.display()
        ));
    }
    Ok(Some(KnowledgeScope {
        kind: KnowledgeScopeKind::DocumentSource,
        member_id: None,
        member_name: None,
        advisor_id: None,
        advisor_name: None,
        language: None,
        language_detection_status: None,
        source_id: Some(source_id),
        source_name: Some(source_name),
        root,
    }))
}

// [KB-PATH-FIX] Build a glob pattern for `rest` relative to a remapped source root. Virtual
// document names (content.md / meta.json) mean "this whole source", so they widen to `**/*`.
fn listing_pattern_for_rest(rest: &str) -> String {
    let rest = rest.trim();
    if rest.is_empty()
        || rest == "content.md"
        || rest == "content.txt"
        || rest == "index.md"
        || rest == "meta.json"
        || rest.contains('*')
    {
        return "**/*".to_string();
    }
    format!("{}/**/*", rest.trim_end_matches('/'))
}

// [KB-PATH-FIX] Production wrapper around `list_pattern_for_scope`. Non-mirrored sources
// (tracked-folder / obsidian-vault) have no `knowledge/docs/doc-source-<id>/` directory at all,
// so listing that canonical path must fall back to the source's real rootPath instead of failing —
// otherwise the authoring agent concludes the material is unreadable and writes without it.
// Returns the glob pattern plus an optional replacement root for the caller to glob against.
fn list_pattern_for_scope_with_remap(
    state: &State<'_, AppState>,
    scope: &KnowledgeScope,
    arguments: &Value,
) -> Result<(String, Option<PathBuf>), String> {
    if let Some(path) = payload_string(arguments, "path").filter(|value| !value.trim().is_empty()) {
        let normalized = normalize_scope_relative_path(scope, &path)?;
        let target = resolve_relative_path(&scope.root, &normalized)?;
        if !target.exists() {
            if let Some((root, rest)) =
                resolve_kb_source_root(state, scope.source_id.as_deref(), &normalized)?
            {
                return Ok((listing_pattern_for_rest(&rest), Some(root)));
            }
            return Err(format!("knowledge path does not exist: {normalized}"));
        }
    }
    Ok((list_pattern_for_scope(scope, arguments)?, None))
}

fn search_pattern_for_scope_with_remap(
    state: &State<'_, AppState>,
    scope: &KnowledgeScope,
    arguments: &Value,
) -> Result<(String, Option<PathBuf>), String> {
    let has_pattern = payload_string(arguments, "pattern")
        .filter(|value| !value.trim().is_empty())
        .is_some();
    if !has_pattern {
        if let Some(path) =
            payload_string(arguments, "path").filter(|value| !value.trim().is_empty())
        {
            let normalized = normalize_scope_relative_path(scope, &path)?;
            let target = resolve_relative_path(&scope.root, &normalized)?;
            if !target.exists() {
                if let Some((root, rest)) =
                    resolve_kb_source_root(state, scope.source_id.as_deref(), &normalized)?
                {
                    return Ok((listing_pattern_for_rest(&rest), Some(root)));
                }
                return Err(format!("knowledge path does not exist: {normalized}"));
            }
        }
    }
    Ok((search_pattern_for_scope(scope, arguments)?, None))
}

fn list_pattern_for_scope(scope: &KnowledgeScope, arguments: &Value) -> Result<String, String> {
    if let Some(path) = payload_string(arguments, "path").filter(|value| !value.trim().is_empty()) {
        let normalized = normalize_scope_relative_path(scope, &path)?;
        let target = resolve_relative_path(&scope.root, &normalized)?;
        if !target.exists() {
            return Err(format!("knowledge path does not exist: {normalized}"));
        }
        return Ok(if target.is_dir() {
            if normalized.is_empty() {
                "**/*".to_string()
            } else {
                format!("{}/**/*", normalized.trim_end_matches('/'))
            }
        } else {
            normalized
        });
    }
    Ok(payload_string(arguments, "pattern")
        .map(|value| normalize_scope_pattern(scope, &value))
        .transpose()?
        .unwrap_or_else(|| "**/*".to_string()))
}

fn search_pattern_for_scope(scope: &KnowledgeScope, arguments: &Value) -> Result<String, String> {
    if let Some(pattern) =
        payload_string(arguments, "pattern").filter(|value| !value.trim().is_empty())
    {
        return normalize_scope_pattern(scope, &pattern);
    }
    if let Some(path) = payload_string(arguments, "path").filter(|value| !value.trim().is_empty()) {
        let normalized = normalize_scope_relative_path(scope, &path)?;
        let target = resolve_relative_path(&scope.root, &normalized)?;
        if !target.exists() {
            return Err(format!("knowledge path does not exist: {normalized}"));
        }
        return Ok(if target.is_dir() {
            if normalized.is_empty() {
                "**/*".to_string()
            } else {
                format!("{}/**/*", normalized.trim_end_matches('/'))
            }
        } else {
            normalized
        });
    }
    Ok("**/*".to_string())
}

fn normalize_scope_pattern(scope: &KnowledgeScope, value: &str) -> Result<String, String> {
    normalize_scope_relative_path(scope, value)
}

fn normalize_scope_relative_path(scope: &KnowledgeScope, value: &str) -> Result<String, String> {
    let normalized = normalize_relative_display(value.trim().to_string());
    if normalized.is_empty() {
        return Ok(String::new());
    }
    match scope.kind {
        KnowledgeScopeKind::Advisor | KnowledgeScopeKind::DocumentSource => Ok(normalized),
        KnowledgeScopeKind::Workspace => {
            let stripped = normalized
                .strip_prefix("knowledge/")
                .or_else(|| normalized.strip_prefix("knowledge\\"))
                .unwrap_or(normalized.as_str())
                .trim_matches('/')
                .to_string();
            if stripped.is_empty() {
                return Ok(String::new());
            }
            if stripped == "knowledge" {
                return Ok(String::new());
            }
            Ok(stripped)
        }
    }
}

fn scope_kind_label(scope: &KnowledgeScope) -> &'static str {
    match scope.kind {
        KnowledgeScopeKind::Advisor => "advisor",
        KnowledgeScopeKind::DocumentSource => "document-source",
        KnowledgeScopeKind::Workspace => "workspace",
    }
}

fn scoped_root_path(scope: &KnowledgeScope) -> String {
    scope.root.display().to_string()
}

fn execute_source_search(
    state: &State<'_, AppState>,
    scope: &KnowledgeScope,
    arguments: &Value,
    query: &str,
) -> Result<Value, String> {
    let source_id = scope
        .source_id
        .as_deref()
        .ok_or_else(|| "document source id is missing".to_string())?;
    // [KB-PATH-FIX] A canonical doc-source path may resolve to the source's real rootPath.
    let (pattern_text, root_override) = search_pattern_for_scope_with_remap(state, scope, arguments)?;
    let pattern = compile_pattern(&pattern_text)?;
    let search_root = root_override.unwrap_or_else(|| scope.root.clone());
    let limit = parse_usize(arguments, "limit", DEFAULT_GREP_LIMIT, MAX_GREP_LIMIT);
    let snippet_chars = parse_usize(arguments, "snippetChars", DEFAULT_SNIPPET_CHARS, 800);
    let query_profile = query_profile::build_query_profile(query);
    let retrieval_mode = parse_retrieval_mode(arguments, &query_profile);
    let query_profile_json = query_profile::query_profile_to_json(&query_profile);
    let indexed_count = document_blocks::count_blocks_for_source(state, source_id)?;
    if indexed_count > 0 {
        let hits = document_blocks::search_blocks(
            state,
            source_id,
            query,
            &pattern,
            limit,
            snippet_chars,
            retrieval_mode,
        )?;
        let (hit_payloads, evidence_pack) = build_hit_payloads_and_evidence_pack(
            state,
            &hits,
            query,
            retrieval_mode,
            &query_profile,
        )?;
        let search_mode = if retrieval_mode == RetrievalMode::Hybrid {
            "indexed-blocks-hybrid"
        } else {
            "indexed-blocks-lexical"
        };
        let mut query_plan =
            build_query_plan_value(&query_profile_json, retrieval_mode, search_mode);
        if let Some(plan) = query_plan.as_object_mut() {
            plan.insert(
                "indexStaleness".to_string(),
                Value::String(index_staleness_label(state)),
            );
        }
        let audit_run_id = retrieval_audit::record_search_run(
            state,
            source_id,
            scope.source_name.as_deref(),
            query,
            search_mode,
            &query_profile_json,
            &query_plan,
            &hit_payloads,
            &evidence_pack,
        )?;
        return Ok(json!({
            "scopeKind": scope_kind_label(scope),
            "memberId": scope.member_id,
            "memberName": scope.member_name,
            "language": scope.language,
            "languageDetectionStatus": scope.language_detection_status,
            "sourceId": scope.source_id,
            "sourceName": scope.source_name,
            "rootPath": scoped_root_path(scope),
            "pattern": pattern_text,
            "query": query,
            "auditRunId": audit_run_id,
            "queryProfile": query_profile_json.clone(),
            "queryPlan": query_plan,
            "searchMode": search_mode,
            "totalMatches": hits.len(),
            "hits": hit_payloads,
            "evidencePack": evidence_pack
        }));
    }

    let query_lower = query.to_lowercase();
    let mut files = collect_matching_files(&search_root, &pattern)?;
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    let mut hits = Vec::<Value>::new();
    for file in files {
        if hits.len() >= limit {
            break;
        }
        if !is_text_file(&file.absolute_path) {
            continue;
        }
        let Ok(content) = fs::read_to_string(&file.absolute_path) else {
            continue;
        };
        for (index, line) in content.lines().enumerate() {
            if !line.to_lowercase().contains(&query_lower) {
                continue;
            }
            let document_id = format!("{}:{}", source_id, file.relative_path);
            hits.push(json!({
                "documentId": document_id,
                "sourceId": source_id,
                "sourceName": scope.source_name,
                "blockId": Value::Null,
                "path": file.relative_path,
                "absolutePath": file.absolute_path.display().to_string(),
                "blockType": Value::Null,
                "sectionPath": Value::Null,
                "page": Value::Null,
                "blockIndex": Value::Null,
                "lineStart": index + 1,
                "lineEnd": index + 1,
                "lineNumber": index + 1,
                "contentOrigin": Value::Null,
                "ocrConfidence": Value::Null,
                "legalMetadata": Value::Null,
                "snippet": truncate_chars(line.trim(), snippet_chars),
            }));
            if hits.len() >= limit {
                break;
            }
        }
    }
    Ok(json!({
        "scopeKind": scope_kind_label(scope),
        "memberId": scope.member_id,
        "memberName": scope.member_name,
        "language": scope.language,
        "languageDetectionStatus": scope.language_detection_status,
        "sourceId": scope.source_id,
        "sourceName": scope.source_name,
        "rootPath": scoped_root_path(scope),
        "pattern": pattern_text,
        "query": query,
        "queryProfile": query_profile_json.clone(),
        "queryPlan": build_query_plan_value(&query_profile_json, retrieval_mode, "filesystem-fallback"),
        "searchMode": "filesystem-fallback",
        "totalMatches": hits.len(),
        "hits": hits
    }))
}

fn build_hit_payloads_and_evidence_pack(
    state: &State<'_, AppState>,
    hits: &[document_blocks::DocumentBlockHit],
    query: &str,
    retrieval_mode: RetrievalMode,
    query_profile: &QueryProfile,
) -> Result<(Vec<Value>, Value), String> {
    let mut hit_payloads = Vec::<Value>::new();
    let mut evidences = Vec::<Value>::new();
    let query_profile_json = query_profile::query_profile_to_json(query_profile);
    for hit in hits {
        let anchors = citation_anchors::anchors_for_block_query(state, &hit.block_id, query, 3)?;
        let visual_evidence = visual_evidence_for_block(
            state,
            hit.visual_unit_id.as_deref(),
            &hit.evidence_refs_json,
        )?;
        let anchor_ids = anchors
            .iter()
            .map(|item| item.anchor_id.clone())
            .collect::<Vec<_>>();
        hit_payloads.push(json!({
            "blockId": hit.block_id,
            "documentId": hit.document_id,
            "sourceId": hit.source_id,
            "sourceName": hit.source_name,
            "rootPath": hit.root_path,
            "path": hit.path,
            "absolutePath": hit.absolute_path,
            "fileExtension": hit.file_extension,
            "title": hit.title,
            "language": hit.language,
            "contentOrigin": hit.content_origin,
            "ocrConfidence": legacy_ocr_confidence(&hit.content_origin, hit.ocr_confidence),
            "visualSource": visual_source_for_block(
                &hit.content_origin,
                &hit.source_id,
                &hit.document_id,
                &hit.path,
                &hit.absolute_path,
                hit.page,
                &hit.block_type,
                hit.visual_unit_id.as_deref(),
                hit.source_document_id.as_deref(),
                &hit.evidence_refs_json
            ),
            "visualSummary": visual_summary_for_hit(&hit.content_origin, &hit.title, &hit.snippet),
            "visualEvidence": visual_evidence,
            "legalMetadata": {
                "jurisdiction": hit.jurisdiction,
                "authority": hit.authority,
                "authorityLevel": hit.authority_level,
                "effectiveDate": hit.effective_date,
                "expiryDate": hit.expiry_date,
                "documentType": hit.document_type,
                "isSuperseded": hit.is_superseded
            },
            "page": hit.page,
            "blockType": hit.block_type,
            "sectionPath": hit.section_path,
            "blockIndex": hit.block_index,
            "lineStart": hit.line_start,
            "lineEnd": hit.line_end,
            "snippet": hit.snippet,
            "anchorIds": anchor_ids,
            "ranking": {
                "lexicalScore": hit.lexical_score,
                "bm25Score": hit.bm25_score,
                "semanticScore": hit.semantic_score,
                "fusionScore": hit.fusion_score,
                "rerankScore": hit.rerank_score,
                "legalScore": hit.legal_score,
                "languageMatchScore": hit.language_match_score,
                "totalScore": hit.total_score
            },
            "retrievalLanes": hit.retrieval_lanes,
        }));
        evidences.push(json!({
            "documentId": hit.document_id,
            "blockId": hit.block_id,
            "path": hit.path,
            "page": hit.page,
            "blockType": hit.block_type,
            "sectionPath": hit.section_path,
            "contentOrigin": hit.content_origin,
            "ocrConfidence": legacy_ocr_confidence(&hit.content_origin, hit.ocr_confidence),
            "visualSource": visual_source_for_block(
                &hit.content_origin,
                &hit.source_id,
                &hit.document_id,
                &hit.path,
                &hit.absolute_path,
                hit.page,
                &hit.block_type,
                hit.visual_unit_id.as_deref(),
                hit.source_document_id.as_deref(),
                &hit.evidence_refs_json
            ),
            "visualEvidence": visual_evidence,
            "legalMetadata": {
                "jurisdiction": hit.jurisdiction,
                "authority": hit.authority,
                "authorityLevel": hit.authority_level,
                "effectiveDate": hit.effective_date,
                "expiryDate": hit.expiry_date,
                "documentType": hit.document_type,
                "isSuperseded": hit.is_superseded
            },
            "retrievalLanes": hit.retrieval_lanes,
            "anchorIds": anchors.iter().map(|item| item.anchor_id.clone()).collect::<Vec<_>>(),
            "anchors": anchors,
            "quotePreview": anchors.first().map(|item| item.quote_text.clone())
        }));
    }
    Ok((
        hit_payloads,
        json!({
            "query": query,
            "queryProfile": query_profile_json.clone(),
        "queryPlan": {
            "intent": query_profile_json.get("intent").cloned().unwrap_or(Value::Null),
            "queryLanguage": query_profile_json.get("language").cloned().unwrap_or(Value::Null),
            "retrievalMode": query_profile::retrieval_mode_label(retrieval_mode),
            "lexicalEngine": "tantivy+sqlite-fts5-bm25",
            "lexicalFallback": "sqlite-like",
            "fusion": if retrieval_mode == RetrievalMode::Hybrid { "weighted-rrf" } else { "none" },
            "languageAwareRanking": true,
            "granularity": query_profile_json.get("granularity").cloned().unwrap_or(Value::Null),
                "citationRequirement": query_profile_json.get("citationRequirement").cloned().unwrap_or(Value::Null),
                "documentTypeHints": query_profile_json.get("documentTypeHints").cloned().unwrap_or(Value::Null),
                "legalBiases": query_profile_json.get("legalBiases").cloned().unwrap_or(Value::Null),
                "rerankers": query_profile.rerankers
            },
            "evidences": evidences,
            "groundingContract": {
                "claimField": "claim",
                "anchorIdsField": "supportingAnchorIds",
                "rule": "Every grounded claim must cite at least one anchorId."
            }
        }),
    ))
}

fn build_query_plan_value(
    query_profile_json: &Value,
    retrieval_mode: RetrievalMode,
    search_mode: &str,
) -> Value {
    json!({
        "retrievalMode": query_profile::retrieval_mode_label(retrieval_mode),
        "searchMode": search_mode,
        "queryLanguage": query_profile_json.get("language").cloned().unwrap_or(Value::Null),
        "lexicalEngine": "tantivy+sqlite-fts5-bm25",
        "lexicalFallback": "sqlite-like",
        "languageAwareRanking": true,
        "granularity": query_profile_json.get("granularity").cloned().unwrap_or(Value::Null),
        "citationRequirement": query_profile_json.get("citationRequirement").cloned().unwrap_or(Value::Null),
        "documentTypeHints": query_profile_json.get("documentTypeHints").cloned().unwrap_or(Value::Null),
        "rerankers": query_profile_json.get("rerankers").cloned().unwrap_or(Value::Null),
    })
}

fn index_staleness_label(state: &State<'_, AppState>) -> String {
    match index_status(state) {
        Ok(status) if status.is_building => "rebuilding".to_string(),
        Ok(status) => status
            .migration_status
            .unwrap_or_else(|| "current".to_string()),
        Err(_) => "unknown".to_string(),
    }
}

fn parse_retrieval_mode(arguments: &Value, query_profile: &QueryProfile) -> RetrievalMode {
    match payload_string(arguments, "retrievalMode").map(|value| value.to_ascii_lowercase()) {
        Some(value) if value == "lexical" => RetrievalMode::Lexical,
        Some(value) if value == "hybrid" => RetrievalMode::Hybrid,
        _ => query_profile.recommended_retrieval_mode,
    }
}

fn resolve_session_advisor_id(
    state: &State<'_, AppState>,
    session_id: Option<&str>,
) -> Option<String> {
    let session_id = session_id?;
    with_store(state, |store| {
        Ok(store
            .chat_sessions
            .iter()
            .find(|item| item.id == session_id)
            .and_then(|item| item.metadata.as_ref().cloned()))
    })
    .ok()
    .flatten()
    .and_then(|metadata| {
        payload_string(&metadata, "advisorId").or_else(|| {
            let context_type = payload_string(&metadata, "contextType");
            if context_type.as_deref() == Some("advisor-discussion") {
                return payload_string(&metadata, "contextId");
            }
            payload_field(&metadata, "advisorIds")
                .and_then(Value::as_array)
                .and_then(|items| {
                    if items.len() == 1 {
                        items
                            .first()
                            .and_then(Value::as_str)
                            .map(|value| value.to_string())
                    } else {
                        None
                    }
                })
        })
    })
}

fn resolve_session_collab_member_id(
    state: &State<'_, AppState>,
    session_id: Option<&str>,
) -> Option<String> {
    let session_id = session_id?;
    with_store(state, |store| {
        if let Some(member_id) = store
            .chat_sessions
            .iter()
            .find(|item| item.id == session_id)
            .and_then(|item| item.metadata.as_ref())
            .and_then(|metadata| {
                payload_string(metadata, "collabMemberId")
                    .or_else(|| payload_string(metadata, "memberId"))
            })
        {
            return Ok(Some(member_id));
        }
        Ok(store
            .collab_members
            .iter()
            .find(|member| member.conversation_id.as_deref() == Some(session_id))
            .map(|member| member.id.clone()))
    })
    .ok()
    .flatten()
}

fn collect_matching_files(root: &Path, pattern: &Pattern) -> Result<Vec<MatchedFile>, String> {
    let mut files = Vec::<MatchedFile>::new();
    if !root.exists() {
        return Ok(files);
    }
    collect_matching_files_recursive(root, root, pattern, &mut files)?;
    Ok(files)
}

fn collect_matching_files_recursive(
    root: &Path,
    current: &Path,
    pattern: &Pattern,
    files: &mut Vec<MatchedFile>,
) -> Result<(), String> {
    let entries = fs::read_dir(current).map_err(|error| error.to_string())?;
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        if path.is_dir() {
            collect_matching_files_recursive(root, &path, pattern, files)?;
            continue;
        }
        if !path.is_file() {
            continue;
        }
        let relative_path = normalize_relative_display(
            path.strip_prefix(root)
                .unwrap_or(path.as_path())
                .display()
                .to_string(),
        );
        if !pattern.matches_with(&relative_path, match_options()) {
            continue;
        }
        let metadata = entry.metadata().map_err(|error| error.to_string())?;
        let updated_at_ms = metadata
            .modified()
            .ok()
            .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
            .map(|value| value.as_millis() as i64)
            .unwrap_or_default();
        files.push(MatchedFile {
            absolute_path: path.clone(),
            relative_path,
            name: path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("")
                .to_string(),
            extension: path
                .extension()
                .and_then(|value| value.to_str())
                .map(|value| value.to_string()),
            size_bytes: metadata.len(),
            updated_at_ms,
        });
    }
    Ok(())
}

fn compile_pattern(pattern: &str) -> Result<Pattern, String> {
    Pattern::new(if pattern.trim().is_empty() {
        "**/*"
    } else {
        pattern
    })
    .map_err(|error| format!("invalid glob pattern: {error}"))
}

fn match_options() -> MatchOptions {
    MatchOptions {
        case_sensitive: false,
        require_literal_separator: false,
        require_literal_leading_dot: false,
    }
}

fn resolve_relative_path(root: &Path, relative_path: &str) -> Result<PathBuf, String> {
    let mut resolved = root.to_path_buf();
    let path = Path::new(relative_path);
    if path.is_absolute() {
        return Err("absolute paths are not allowed".to_string());
    }
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => resolved.push(part),
            Component::ParentDir => {
                return Err("parent directory traversal is not allowed".to_string());
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err("absolute paths are not allowed".to_string());
            }
        }
    }
    Ok(resolved)
}

fn normalize_relative_display(value: String) -> String {
    value.replace('\\', "/")
}

fn parse_usize(arguments: &Value, key: &str, default: usize, max: usize) -> usize {
    payload_field(arguments, key)
        .and_then(|value| match value {
            Value::Number(number) => number.as_u64().map(|item| item as usize),
            Value::String(text) => text.trim().parse::<usize>().ok(),
            _ => None,
        })
        .map(|value| value.clamp(0, max))
        .unwrap_or(default)
}

fn is_text_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|value| value.to_str()).map(|value| value.to_ascii_lowercase()),
        Some(ext)
            if matches!(
                ext.as_str(),
                "md" | "markdown" | "txt" | "json" | "yaml" | "yml" | "csv" | "tsv" | "srt"
                    | "vtt" | "html" | "htm" | "xml" | "js" | "ts" | "jsx" | "tsx"
            )
    )
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let chars = value.chars().collect::<Vec<_>>();
    if chars.len() <= max_chars {
        return value.to_string();
    }
    chars.into_iter().take(max_chars).collect::<String>()
}

fn legacy_ocr_confidence(content_origin: &str, confidence: Option<f64>) -> Option<f64> {
    if content_origin == "ocr" {
        confidence
    } else {
        None
    }
}

fn visual_source_for_block(
    content_origin: &str,
    source_id: &str,
    document_id: &str,
    path: &str,
    absolute_path: &str,
    page: Option<i64>,
    block_type: &str,
    visual_unit_id: Option<&str>,
    source_document_id: Option<&str>,
    evidence_refs_json: &str,
) -> Option<Value> {
    if content_origin != "visual_llm" && !block_type.starts_with("image.") {
        return None;
    }
    Some(json!({
        "unitId": visual_unit_id,
        "unitKind": if page.is_some() { "pdf_page" } else { "image_file" },
        "sourceId": source_id,
        "documentId": document_id,
        "sourceDocumentId": source_document_id
            .unwrap_or_else(|| document_id.split("#page=").next().unwrap_or(document_id)),
        "path": path,
        "absolutePath": absolute_path,
        "pageNumber": page,
        "blockType": block_type,
        "evidenceRefs": parse_evidence_refs(evidence_refs_json)
    }))
}

fn parse_evidence_refs(raw: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(raw).unwrap_or_default()
}

fn visual_evidence_for_block(
    state: &State<'_, AppState>,
    visual_unit_id: Option<&str>,
    evidence_refs_json: &str,
) -> Result<Vec<Value>, String> {
    let Some(unit_id) = visual_unit_id.filter(|value| !value.trim().is_empty()) else {
        return Ok(Vec::new());
    };
    let wanted_refs = parse_evidence_refs(evidence_refs_json);
    let conn = open_catalog_connection(state)?;
    let mut stmt = conn
        .prepare(
            r#"
            SELECT evidence_id, unit_id, source_document_id, document_id, block_id,
                   projection_id, page_number, bbox_json, label, text
            FROM knowledge_visual_evidence
            WHERE unit_id = ?1
            ORDER BY evidence_id ASC
            "#,
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map(params![unit_id], |row| {
            let evidence_id: String = row.get(0)?;
            let bbox_json: Option<String> = row.get(7)?;
            let bbox = bbox_json
                .as_deref()
                .and_then(|value| serde_json::from_str::<Value>(value).ok())
                .unwrap_or(Value::Null);
            Ok(json!({
                "evidenceId": evidence_id,
                "unitId": row.get::<_, String>(1)?,
                "sourceDocumentId": row.get::<_, String>(2)?,
                "documentId": row.get::<_, String>(3)?,
                "blockId": row.get::<_, Option<String>>(4)?,
                "projectionId": row.get::<_, Option<String>>(5)?,
                "pageNumber": row.get::<_, Option<i64>>(6)?,
                "bbox": bbox,
                "label": row.get::<_, Option<String>>(8)?,
                "text": row.get::<_, String>(9)?,
            }))
        })
        .map_err(|error| error.to_string())?;
    let mut evidence = Vec::new();
    for row in rows {
        let item = row.map_err(|error| error.to_string())?;
        if wanted_refs.is_empty()
            || item
                .get("evidenceId")
                .and_then(Value::as_str)
                .is_some_and(|value| {
                    wanted_refs
                        .iter()
                        .any(|wanted| value.ends_with(&format!(":{wanted}")) || value == wanted)
                })
        {
            evidence.push(item);
        }
    }
    Ok(evidence)
}

fn visual_summary_for_hit(
    content_origin: &str,
    title: &Option<String>,
    snippet: &str,
) -> Option<Value> {
    if content_origin != "visual_llm" {
        return None;
    }
    Some(json!({
        "title": title,
        "snippet": snippet
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge_index::query_profile::build_query_profile;

    fn workspace_scope() -> KnowledgeScope {
        KnowledgeScope {
            kind: KnowledgeScopeKind::Workspace,
            member_id: None,
            member_name: None,
            advisor_id: None,
            advisor_name: None,
            language: None,
            language_detection_status: None,
            source_id: None,
            source_name: None,
            root: PathBuf::from("/tmp/workspace/knowledge"),
        }
    }

    #[test]
    fn normalize_workspace_knowledge_path_strips_prefix() {
        let normalized = normalize_scope_relative_path(
            &workspace_scope(),
            "knowledge/redbook/knowledge-123/meta.json",
        )
        .unwrap();
        assert_eq!(normalized, "redbook/knowledge-123/meta.json");
    }

    #[test]
    fn normalize_workspace_knowledge_root_to_empty_relative_path() {
        let normalized = normalize_scope_relative_path(&workspace_scope(), "knowledge").unwrap();
        assert_eq!(normalized, "");
    }

    #[test]
    fn list_pattern_uses_directory_path_as_glob_root() {
        let unique = format!(
            "redbox-knowledge-search-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique).join("knowledge");
        let temp_root = root.parent().unwrap_or(root.as_path()).to_path_buf();
        let folder = root.join("redbook").join("knowledge-123");
        fs::create_dir_all(&folder).unwrap();
        let scope = KnowledgeScope {
            kind: KnowledgeScopeKind::Workspace,
            member_id: None,
            member_name: None,
            advisor_id: None,
            advisor_name: None,
            language: None,
            language_detection_status: None,
            source_id: None,
            source_name: None,
            root,
        };
        let arguments = json!({
            "path": "knowledge/redbook/knowledge-123"
        });
        let pattern = list_pattern_for_scope(&scope, &arguments).unwrap();
        assert_eq!(pattern, "redbook/knowledge-123/**/*");
        let _ = fs::remove_dir_all(temp_root);
    }

    #[test]
    fn collect_matching_files_returns_empty_for_missing_workspace_knowledge_root() {
        let unique = format!(
            "redbox-missing-knowledge-root-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique).join("knowledge");
        let pattern = compile_pattern("**/*").unwrap();

        let files = collect_matching_files(&root, &pattern).unwrap();

        assert!(files.is_empty());
    }

    #[test]
    fn parse_retrieval_mode_defaults_to_query_profile_recommendation() {
        let statute_profile = build_query_profile("请引用民法典第577条原文");
        assert_eq!(
            parse_retrieval_mode(&json!({}), &statute_profile),
            RetrievalMode::Lexical
        );

        let synthesis_profile =
            build_query_profile("compare contract breach remedy across multiple documents");
        assert_eq!(
            parse_retrieval_mode(&json!({}), &synthesis_profile),
            RetrievalMode::Hybrid
        );
    }

    #[test]
    fn explicit_retrieval_mode_overrides_query_profile_recommendation() {
        let profile = build_query_profile("请引用民法典第577条原文");
        assert_eq!(
            parse_retrieval_mode(&json!({ "retrievalMode": "hybrid" }), &profile),
            RetrievalMode::Hybrid
        );
    }

    #[test]
    fn visual_blocks_do_not_expose_legacy_ocr_confidence() {
        assert_eq!(legacy_ocr_confidence("visual_llm", Some(0.98)), None);
        assert_eq!(legacy_ocr_confidence("ocr", Some(0.76)), Some(0.76));
    }

    #[test]
    fn visual_source_metadata_points_to_original_unit_and_page() {
        let source = visual_source_for_block(
            "visual_llm",
            "source-1",
            "source-1:scans/contract.pdf#page=3",
            "scans/contract.pdf#page=3",
            "/tmp/contract.pdf",
            Some(3),
            "image.visible_text",
            Some("unit-3"),
            Some("source-1:scans/contract.pdf"),
            r#"["fact_visible_text"]"#,
        )
        .expect("visual source");

        assert_eq!(source["unitId"], json!("unit-3"));
        assert_eq!(source["unitKind"], json!("pdf_page"));
        assert_eq!(
            source["sourceDocumentId"],
            json!("source-1:scans/contract.pdf")
        );
        assert_eq!(source["pageNumber"], json!(3));
        assert_eq!(source["evidenceRefs"], json!(["fact_visible_text"]));
    }

    // ---------------------------------------------------------------------
    // [KB-PATH-FIX] Real-data regression coverage.
    //
    // The shipped bug was silent: `sources.json` stores ids WITH the `doc-source-` prefix, while
    // the remap stripped that prefix before matching, so it never hit and the authoring agent
    // could not read tracked-folder / obsidian-vault material at all. A hand-written Python
    // replica "passed" because it inherited the same wrong assumption — these tests run the REAL
    // Rust resolution code against the REAL sources.json instead.
    //
    // They skip cleanly when no live space is present so the suite stays portable.
    // ---------------------------------------------------------------------

    fn real_document_sources() -> Option<Vec<DocumentKnowledgeSourceRecord>> {
        let path = std::env::var("BEAV_TEST_SOURCES_JSON")
            .ok()
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(r"C:\Users\HP\.beav\spaces\test_demo-space-1792119246725")
                    .join("knowledge")
                    .join("docs")
                    .join("sources.json")
            });
        if !path.exists() {
            eprintln!("[skip] real sources.json not found at {}", path.display());
            return None;
        }
        let raw = fs::read_to_string(&path).ok()?;
        match serde_json::from_str::<Vec<DocumentKnowledgeSourceRecord>>(&raw) {
            Ok(sources) => Some(sources),
            Err(error) => {
                eprintln!("[skip] cannot parse {}: {error}", path.display());
                None
            }
        }
    }

    fn sample_source(id: &str, kind: &str, root_path: &str) -> DocumentKnowledgeSourceRecord {
        serde_json::from_value(json!({
            "id": id,
            "kind": kind,
            "name": "test",
            "rootPath": root_path,
            "locked": false,
            "indexing": false,
            "indexError": Value::Null,
            "fileCount": 1,
            "sampleFiles": [],
            "createdAt": "0",
            "updatedAt": "0",
        }))
        .expect("sample source record")
    }

    #[test]
    fn find_document_source_matches_prefixed_and_bare_ids() {
        let sources = vec![sample_source(
            "doc-source-1792121765191",
            "tracked-folder",
            r"C:\material",
        )];

        // Both forms must resolve; a missing one silently disables every remap.
        assert!(find_document_source(&sources, "doc-source-1792121765191").is_some());
        assert!(find_document_source(&sources, "1792121765191").is_some());
        assert!(find_document_source(&sources, "1792999999999").is_none());
    }

    #[test]
    fn kb_path_fix_reads_real_material_text_from_non_mirrored_sources() {
        let Some(sources) = real_document_sources() else {
            return;
        };
        let non_copied: Vec<_> = sources.iter().filter(|s| s.kind != "copied-file").collect();
        assert!(
            !non_copied.is_empty(),
            "expected at least one non-copied (tracked-folder / obsidian-vault) source"
        );

        for source in &non_copied {
            for path in [
                format!("docs/{}/content.md", source.id),
                format!("{}/content.md", source.id),
                format!("doc-source-{}/content.md", source.id.trim_start_matches("doc-source-")),
            ] {
                let resolved = resolve_kb_source_canonical_from_sources(&sources, None, &path)
                    .unwrap_or_else(|error| panic!("{path} errored: {error}"));
                match resolved {
                    Some(KbResolved::Text(text)) => assert!(
                        text.len() > 200,
                        "{path} returned only {} chars — material is effectively unreadable",
                        text.len()
                    ),
                    _ => panic!("{path} did not resolve to material text"),
                }
            }
        }
    }

    #[test]
    fn kb_path_fix_reads_a_specific_real_material_file_by_name() {
        let Some(sources) = real_document_sources() else {
            return;
        };
        let Some(source) = sources
            .iter()
            .find(|item| item.root_path.contains("beav_demo_material_1"))
        else {
            eprintln!("[skip] demo material source absent");
            return;
        };

        let path = format!("docs/{}/知识库为什么建了就荒.md", source.id);
        let resolved = resolve_kb_source_canonical_from_sources(&sources, None, &path)
            .unwrap_or_else(|error| panic!("{path} errored: {error}"));
        let Some(KbResolved::File(file)) = resolved else {
            panic!("{path} did not resolve to a real file");
        };
        assert!(file.is_file(), "{} is not a file", file.display());
        let text = fs::read_to_string(&file).expect("read material");
        assert!(
            text.contains("数字坟场"),
            "expected the real material phrase, got: {}",
            text.chars().take(120).collect::<String>()
        );
    }

    #[test]
    fn kb_path_fix_listing_resolves_real_source_root() {
        let Some(sources) = real_document_sources() else {
            return;
        };

        for source in sources.iter().filter(|s| s.kind != "copied-file") {
            let path = format!("docs/{}", source.id);
            let Some((root, rest)) = resolve_kb_source_root_from_sources(&sources, None, &path)
            else {
                panic!("{path} should remap to the real source root");
            };
            assert!(root.is_dir(), "{} is not a dir", root.display());
            assert!(rest.is_empty(), "unexpected remainder: {rest}");
            assert_eq!(listing_pattern_for_rest(&rest), "**/*");

            // The remapped root must actually enumerate material, otherwise list/search still
            // look empty and the agent concludes the knowledge base has nothing.
            let pattern = compile_pattern(&listing_pattern_for_rest(&rest)).unwrap();
            let files = collect_matching_files(&root, &pattern).unwrap();
            assert!(
                !files.is_empty(),
                "{path} remapped to {} but found no files",
                root.display()
            );
        }
    }
}
