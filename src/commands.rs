use crate::diff::{apply_selected_hunks, get_hunks, Hunk, HunkSelection};
use crate::spec::{Action, DefaultAction, FileSpec, Spec};
use anyhow::{Context, Result};
use clap::ValueEnum;
use hunkset::{evaluate, OccurrenceKey, SelectableUnit};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::Path;
use std::process::Command;
use walkdir::WalkDir;

const JJ_HUNK_TOOL_ARG: &str = "--tool=jj-hunk";
const JJ_HUNK_PROGRAM_KEY: &str = "merge-tools.jj-hunk.program";
const JJ_HUNK_EDIT_ARGS_KEY: &str = "merge-tools.jj-hunk.edit-args";
const JJ_HUNK_LIST_REQUEST: &str = "JJ_HUNK_LIST_REQUEST";
const JJ_HUNK_LIST_OUTPUT: &str = "JJ_HUNK_LIST_OUTPUT";
const JJ_HUNK_LIST_TOOL: &str = "jj-hunk-list";

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
pub enum ListFormat {
    Json,
    Yaml,
    Text,
}

impl Default for ListFormat {
    fn default() -> Self {
        Self::Json
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
pub enum ListGrouping {
    None,
    Directory,
    Extension,
    Status,
}

impl Default for ListGrouping {
    fn default() -> Self {
        Self::None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
pub enum BinaryMode {
    Skip,
    Mark,
    Include,
}

impl Default for BinaryMode {
    fn default() -> Self {
        Self::Mark
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ListMode {
    Full,
    Files,
    SpecTemplate,
}

impl Default for ListMode {
    fn default() -> Self {
        Self::Full
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListOptions {
    pub rev: Option<String>,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub group: ListGrouping,
    pub format: ListFormat,
    pub mode: ListMode,
    pub spec: Option<String>,
    pub spec_file: Option<String>,
    pub query: Option<String>,
    pub binary: BinaryMode,
    pub max_bytes: Option<usize>,
    pub max_lines: Option<usize>,
}

impl Default for ListOptions {
    fn default() -> Self {
        Self {
            rev: None,
            include: Vec::new(),
            exclude: Vec::new(),
            group: ListGrouping::default(),
            format: ListFormat::default(),
            mode: ListMode::default(),
            spec: None,
            spec_file: None,
            query: None,
            binary: BinaryMode::default(),
            max_bytes: None,
            max_lines: None,
        }
    }
}

impl From<Option<&str>> for ListOptions {
    fn from(rev: Option<&str>) -> Self {
        Self {
            rev: rev.map(str::to_string),
            ..Self::default()
        }
    }
}

#[derive(Debug, Serialize)]
struct ListOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    files: Option<Vec<FileEntry>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    groups: Option<Vec<ListGroup>>,
}

#[derive(Debug, Serialize)]
struct ListGroup {
    name: String,
    files: Vec<FileEntry>,
}

#[derive(Debug, Serialize)]
struct FileEntry {
    path: String,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    rename: Option<RenameInfo>,
    hunks: Vec<Hunk>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    file_units: Vec<FileUnitOutput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    binary: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    truncated: Option<bool>,
}

#[derive(Clone, Debug, Serialize)]
struct FileUnitOutput {
    kind: FileUnitKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    old_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    new_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    removed: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    added: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
enum FileUnitKind {
    Creation,
    Deletion,
    Rename,
    Mode,
    Binary,
}

#[derive(Debug, Serialize, Clone)]
struct RenameInfo {
    from: String,
    to: String,
}

#[derive(Debug, Serialize)]
struct ListSummaryOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    files: Option<Vec<FileSummary>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    groups: Option<Vec<ListSummaryGroup>>,
}

#[derive(Debug, Serialize)]
struct ListSummaryGroup {
    name: String,
    files: Vec<FileSummary>,
}

#[derive(Debug, Serialize)]
struct FileSummary {
    path: String,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    rename: Option<RenameInfo>,
    hunk_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    binary: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    truncated: Option<bool>,
}

#[derive(Debug, Serialize)]
struct SpecTemplateOutput {
    files: HashMap<String, SpecTemplateEntry>,
    default: String,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum SpecTemplateEntry {
    Ids { ids: Vec<String> },
    Action { action: String },
}

#[derive(Debug, Serialize, Deserialize)]
struct DiffSummaryEntry {
    status: String,
    path: String,
    #[serde(default)]
    source: String,
    #[serde(default)]
    target: String,
    #[serde(default)]
    source_type: String,
    #[serde(default)]
    target_type: String,
    #[serde(default)]
    source_executable: bool,
    #[serde(default)]
    target_executable: bool,
    #[serde(default)]
    source_conflict: bool,
    #[serde(default)]
    target_conflict: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct ListRequest {
    options: ListOptions,
    summary_entries: Vec<DiffSummaryEntry>,
}

/// List hunks in current working copy or a specific revision
pub fn list<T>(options: T) -> Result<()>
where
    T: Into<ListOptions>,
{
    let mut options = options.into();
    validate_query_options(&options)?;
    options.spec = resolve_optional_spec(options.spec.as_deref(), options.spec_file.as_deref())?;
    options.spec_file = None;

    let summary_entries = read_diff_summary(options.rev.as_deref())?;
    let rev = options.rev.clone();
    let request = ListRequest {
        options,
        summary_entries,
    };
    let rendered = run_materialized_list(&request, rev.as_deref())?;
    print!("{rendered}");
    Ok(())
}

pub fn list_materialized(left: &str, right: &str) -> Result<()> {
    let request_path =
        std::env::var(JJ_HUNK_LIST_REQUEST).context("Missing internal list request path")?;
    let output_path =
        std::env::var(JJ_HUNK_LIST_OUTPUT).context("Missing internal list output path")?;
    let request: ListRequest =
        serde_json::from_slice(&fs::read(&request_path).with_context(|| {
            format!("Failed to read internal list request from {request_path}")
        })?)
        .context("Failed to parse internal list request")?;

    let rendered = render_materialized_list(request, Path::new(left), Path::new(right))?;
    fs::write(&output_path, rendered)
        .with_context(|| format!("Failed to write internal list output to {output_path}"))?;
    Ok(())
}

fn render_materialized_list(
    request: ListRequest,
    before_root: &Path,
    after_root: &Path,
) -> Result<String> {
    let options = request.options;
    let spec = options.spec.as_deref().map(Spec::from_str).transpose()?;

    let include = normalize_patterns(&options.include);
    let exclude = normalize_patterns(&options.exclude);

    let mut files = Vec::new();
    let query = options.query.as_deref();

    for entry in request.summary_entries {
        let path = primary_path(&entry);
        if path.is_empty() {
            continue;
        }

        if !should_include_entry(&entry, &include, &exclude) {
            continue;
        }

        let decision = spec_decision(spec.as_ref(), &path);
        if matches!(decision, SpecDecision::Skip) {
            continue;
        }

        let file_paths = file_paths_for_entry(&entry, &path);
        let before_bytes = read_materialized_file(before_root, file_paths.before.as_deref())?;
        let after_bytes = read_materialized_file(after_root, file_paths.after.as_deref())?;

        let is_binary = is_binary_data(&before_bytes) || is_binary_data(&after_bytes);
        let binary_changed = is_binary && before_bytes != after_bytes;
        let mode_changed = entry.source_executable != entry.target_executable;
        if query.is_some() {
            validate_query_entry(&entry, &path)?;
        }
        if query.is_none() && is_binary && options.binary == BinaryMode::Skip {
            continue;
        }

        let should_diff = if query.is_some() {
            !is_binary
        } else {
            !(is_binary && options.binary == BinaryMode::Mark)
        };
        let (before_text, before_truncated) = if should_diff && query.is_none() {
            truncate_text(
                &String::from_utf8_lossy(&before_bytes),
                options.max_bytes,
                options.max_lines,
            )
        } else if should_diff {
            (String::from_utf8_lossy(&before_bytes).into_owned(), false)
        } else {
            (String::new(), false)
        };
        let (after_text, after_truncated) = if should_diff && query.is_none() {
            truncate_text(
                &String::from_utf8_lossy(&after_bytes),
                options.max_bytes,
                options.max_lines,
            )
        } else if should_diff {
            (String::from_utf8_lossy(&after_bytes).into_owned(), false)
        } else {
            (String::new(), false)
        };

        let mut hunks = if should_diff
            && !(query.is_some() && matches!(entry.status.as_str(), "added" | "removed"))
        {
            get_hunks(&before_text, &after_text)
        } else {
            Vec::new()
        };

        if let SpecDecision::KeepSelection(selection) = &decision {
            hunks = filter_hunks(hunks, selection);
        }

        let file_units = if query.is_some() {
            prepare_file_units(
                &entry,
                &file_paths,
                &before_text,
                &after_text,
                binary_changed,
                mode_changed,
            )
        } else {
            Vec::new()
        };

        if hunks.is_empty() && !is_binary && file_units.is_empty() {
            continue;
        }

        let rename = rename_info(&entry);
        let truncated = before_truncated || after_truncated;

        files.push(FileEntry {
            path,
            status: entry.status.clone(),
            rename,
            hunks,
            file_units,
            binary: if is_binary { Some(true) } else { None },
            truncated: if truncated { Some(true) } else { None },
        });
    }

    if let Some(query) = query {
        filter_files_by_query(&mut files, query, options.max_bytes, options.max_lines)?;
    }

    match options.mode {
        ListMode::Full => {
            let output = if options.group == ListGrouping::None {
                ListOutput {
                    files: Some(files),
                    groups: None,
                }
            } else {
                let groups = group_files(files, options.group);
                ListOutput {
                    files: None,
                    groups: Some(groups),
                }
            };

            match options.format {
                ListFormat::Json => Ok(format!("{}\n", serde_json::to_string_pretty(&output)?)),
                ListFormat::Yaml => Ok(format!("{}\n", serde_yaml::to_string(&output)?)),
                ListFormat::Text => Ok(render_text_output(&output)),
            }
        }
        ListMode::Files => {
            let summary = build_summary_output(files, options.group);
            match options.format {
                ListFormat::Json => Ok(format!("{}\n", serde_json::to_string_pretty(&summary)?)),
                ListFormat::Yaml => Ok(format!("{}\n", serde_yaml::to_string(&summary)?)),
                ListFormat::Text => Ok(render_text_summary_output(&summary)),
            }
        }
        ListMode::SpecTemplate => {
            if matches!(options.format, ListFormat::Text) {
                anyhow::bail!("--spec-template does not support text output (use json or yaml)");
            }
            let template = build_spec_template(files);
            match options.format {
                ListFormat::Json => Ok(format!("{}\n", serde_json::to_string_pretty(&template)?)),
                ListFormat::Yaml => Ok(format!("{}\n", serde_yaml::to_string(&template)?)),
                ListFormat::Text => unreachable!(),
            }
        }
    }
}

fn validate_query_options(options: &ListOptions) -> Result<()> {
    let Some(query) = options.query.as_deref() else {
        return Ok(());
    };
    if options.spec.is_some() || options.spec_file.is_some() {
        anyhow::bail!("--query cannot be combined with --spec or --spec-file");
    }
    if !options.include.is_empty() || !options.exclude.is_empty() {
        anyhow::bail!("--query cannot be combined with --include or --exclude");
    }
    if options.mode != ListMode::Full {
        anyhow::bail!("--query supports full list output only");
    }
    evaluate(query, &[]).context("Invalid hunkset query")?;
    Ok(())
}

fn validate_query_entry(entry: &DiffSummaryEntry, path: &str) -> Result<()> {
    if entry.source_conflict || entry.target_conflict {
        anyhow::bail!("query preview does not support conflicted files: {path}");
    }
    if entry.status == "copied" {
        anyhow::bail!("query preview does not support copied files: {path}");
    }

    let supported_types = match entry.status.as_str() {
        "added" => entry.target_type == "file",
        "removed" => entry.source_type == "file",
        "modified" | "renamed" => entry.source_type == "file" && entry.target_type == "file",
        _ => false,
    };
    if !supported_types {
        anyhow::bail!("query preview does not support special-file changes: {path}");
    }
    Ok(())
}

fn prepare_file_units(
    entry: &DiffSummaryEntry,
    paths: &FilePaths,
    before_text: &str,
    after_text: &str,
    binary_changed: bool,
    mode_changed: bool,
) -> Vec<FileUnitOutput> {
    let mut units = Vec::new();
    if entry.status == "renamed" {
        units.push(FileUnitOutput {
            kind: FileUnitKind::Rename,
            old_path: paths.before.clone(),
            new_path: paths.after.clone(),
            removed: None,
            added: None,
        });
    }
    if mode_changed && matches!(entry.status.as_str(), "modified" | "renamed") {
        units.push(FileUnitOutput {
            kind: FileUnitKind::Mode,
            old_path: paths.before.clone(),
            new_path: paths.after.clone(),
            removed: None,
            added: None,
        });
    }
    if binary_changed {
        units.push(FileUnitOutput {
            kind: FileUnitKind::Binary,
            old_path: paths.before.clone(),
            new_path: paths.after.clone(),
            removed: None,
            added: None,
        });
    } else if entry.status == "added" {
        units.push(FileUnitOutput {
            kind: FileUnitKind::Creation,
            old_path: None,
            new_path: paths.after.clone(),
            removed: None,
            added: Some(after_text.to_owned()),
        });
    } else if entry.status == "removed" {
        units.push(FileUnitOutput {
            kind: FileUnitKind::Deletion,
            old_path: paths.before.clone(),
            new_path: None,
            removed: Some(before_text.to_owned()),
            added: None,
        });
    }
    units
}

fn filter_files_by_query(
    files: &mut Vec<FileEntry>,
    query: &str,
    max_bytes: Option<usize>,
    max_lines: Option<usize>,
) -> Result<()> {
    let occurrences = prepare_query_occurrences(files);
    let units = occurrences
        .iter()
        .map(|occurrence| occurrence.unit.clone())
        .collect::<Vec<_>>();
    let selected_indices = evaluate(query, &units)
        .context("Invalid hunkset query")?
        .into_iter()
        .map(OccurrenceKey::index)
        .collect::<HashSet<_>>();
    let selected_locations = occurrences
        .iter()
        .enumerate()
        .filter_map(|(occurrence_index, occurrence)| {
            selected_indices
                .contains(&occurrence_index)
                .then_some((occurrence.file_index, occurrence.location))
        })
        .collect::<HashSet<_>>();

    for (file_index, file) in files.iter_mut().enumerate() {
        let mut display_truncated = false;
        let mut hunk_index = 0usize;
        file.hunks.retain_mut(|hunk| {
            let keep =
                selected_locations.contains(&(file_index, PreparedLocation::Hunk(hunk_index)));
            hunk_index += 1;
            if keep {
                let (removed, removed_truncated) =
                    truncate_text(&hunk.removed, max_bytes, max_lines);
                let (added, added_truncated) = truncate_text(&hunk.added, max_bytes, max_lines);
                hunk.removed = removed;
                hunk.added = added;
                display_truncated |= removed_truncated || added_truncated;
            }
            keep
        });
        let mut file_unit_index = 0usize;
        file.file_units.retain_mut(|unit| {
            let keep = selected_locations
                .contains(&(file_index, PreparedLocation::FileUnit(file_unit_index)));
            file_unit_index += 1;
            if keep {
                display_truncated |= truncate_file_unit(unit, max_bytes, max_lines);
            }
            keep
        });
        if !file
            .file_units
            .iter()
            .any(|unit| unit.kind == FileUnitKind::Rename)
        {
            file.rename = None;
        }
        if !file
            .file_units
            .iter()
            .any(|unit| unit.kind == FileUnitKind::Binary)
        {
            file.binary = None;
        }
        if display_truncated {
            file.truncated = Some(true);
        }
    }
    files.retain(|file| !file.hunks.is_empty() || !file.file_units.is_empty());
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum PreparedLocation {
    Hunk(usize),
    FileUnit(usize),
}

struct PreparedOccurrence {
    unit: SelectableUnit,
    file_index: usize,
    location: PreparedLocation,
}

fn prepare_query_occurrences(files: &[FileEntry]) -> Vec<PreparedOccurrence> {
    let mut occurrences = Vec::new();
    for (file_index, file) in files.iter().enumerate() {
        for (file_unit_index, output) in file.file_units.iter().enumerate() {
            occurrences.push(PreparedOccurrence {
                unit: selectable_file_unit(output),
                file_index,
                location: PreparedLocation::FileUnit(file_unit_index),
            });
        }
        for (hunk_index, hunk) in file.hunks.iter().enumerate() {
            let (old_path, new_path) = file
                .rename
                .as_ref()
                .map(|rename| (rename.from.as_str(), rename.to.as_str()))
                .unwrap_or((file.path.as_str(), file.path.as_str()));
            occurrences.push(PreparedOccurrence {
                unit: SelectableUnit::text(old_path, new_path, &hunk.removed, &hunk.added)
                    .expect("materialized diff paths are nonempty"),
                file_index,
                location: PreparedLocation::Hunk(hunk_index),
            });
        }
    }
    occurrences
}

fn selectable_file_unit(output: &FileUnitOutput) -> SelectableUnit {
    let old_path = output.old_path.as_deref();
    let new_path = output.new_path.as_deref();
    match output.kind {
        FileUnitKind::Creation => SelectableUnit::creation(
            new_path.expect("creation has a new path"),
            output.added.as_deref().unwrap_or_default(),
        ),
        FileUnitKind::Deletion => SelectableUnit::deletion(
            old_path.expect("deletion has an old path"),
            output.removed.as_deref().unwrap_or_default(),
        ),
        FileUnitKind::Rename => SelectableUnit::rename(
            old_path.expect("rename has an old path"),
            new_path.expect("rename has a new path"),
        ),
        FileUnitKind::Mode => SelectableUnit::mode(
            old_path.expect("mode change has an old path"),
            new_path.expect("mode change has a new path"),
        ),
        FileUnitKind::Binary => match (old_path, new_path) {
            (Some(old_path), Some(new_path)) => SelectableUnit::binary_existing(old_path, new_path),
            (None, Some(new_path)) => SelectableUnit::binary_creation(new_path),
            (Some(old_path), None) => SelectableUnit::binary_deletion(old_path),
            (None, None) => unreachable!("binary change has at least one path"),
        },
    }
    .expect("prepared query paths are nonempty")
}

fn truncate_file_unit(
    unit: &mut FileUnitOutput,
    max_bytes: Option<usize>,
    max_lines: Option<usize>,
) -> bool {
    let mut truncated = false;
    if let Some(removed) = &mut unit.removed {
        let (limited, was_truncated) = truncate_text(removed, max_bytes, max_lines);
        *removed = limited;
        truncated |= was_truncated;
    }
    if let Some(added) = &mut unit.added {
        let (limited, was_truncated) = truncate_text(added, max_bytes, max_lines);
        *added = limited;
        truncated |= was_truncated;
    }
    truncated
}

const SUMMARY_TEMPLATE: &str = r#""{\"status\":" ++ self.status().escape_json() ++ ",\"path\":" ++ self.path().display().escape_json() ++ ",\"source\":" ++ self.source().path().display().escape_json() ++ ",\"target\":" ++ self.target().path().display().escape_json() ++ ",\"source_type\":" ++ self.source().file_type().escape_json() ++ ",\"target_type\":" ++ self.target().file_type().escape_json() ++ ",\"source_executable\":" ++ self.source().executable() ++ ",\"target_executable\":" ++ self.target().executable() ++ ",\"source_conflict\":" ++ self.source().conflict() ++ ",\"target_conflict\":" ++ self.target().conflict() ++ "}\n""#;

struct FilePaths {
    before: Option<String>,
    after: Option<String>,
}

enum SpecDecision {
    Skip,
    KeepAll,
    KeepSelection(HunkSelection),
}

fn resolve_optional_spec(spec: Option<&str>, spec_file: Option<&str>) -> Result<Option<String>> {
    if spec.is_none() && spec_file.is_none() {
        return Ok(None);
    }

    Ok(Some(resolve_spec_input(spec, spec_file)?))
}

fn run_materialized_list(request: &ListRequest, revset: Option<&str>) -> Result<String> {
    let temp_dir = std::env::temp_dir();
    let process_id = std::process::id();
    let request_path = temp_dir.join(format!("jj-hunk-{process_id}.list-request"));
    let output_path = temp_dir.join(format!("jj-hunk-{process_id}.list-output"));

    fs::remove_file(&request_path).ok();
    fs::remove_file(&output_path).ok();
    fs::write(&request_path, serde_json::to_vec(request)?)
        .with_context(|| format!("Failed to write list request to {}", request_path.display()))?;

    let result = (|| {
        let program = std::env::current_exe()
            .context("Failed to determine current jj-hunk executable path")?;
        let program = program
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("jj-hunk executable path is not valid UTF-8"))?;

        let mut args = vec![
            "--config".to_string(),
            format!(
                "merge-tools.{JJ_HUNK_LIST_TOOL}.program={}",
                toml_string(program)
            ),
            "--config".to_string(),
            format!(
                r#"merge-tools.{JJ_HUNK_LIST_TOOL}.diff-args=["list-materialized", "$left", "$right"]"#
            ),
            "--config".to_string(),
            format!(r#"merge-tools.{JJ_HUNK_LIST_TOOL}.diff-invocation-mode="dir""#),
            "--config".to_string(),
            format!("merge-tools.{JJ_HUNK_LIST_TOOL}.diff-do-chdir=false"),
            "diff".to_string(),
            format!("--tool={JJ_HUNK_LIST_TOOL}"),
        ];
        if let Some(revset) = revset {
            args.push("-r".to_string());
            args.push(revset.to_string());
        }

        let output = Command::new("jj")
            .args(&args)
            .env(JJ_HUNK_LIST_REQUEST, &request_path)
            .env(JJ_HUNK_LIST_OUTPUT, &output_path)
            .output()
            .context("Failed to run jj diff")?;

        if !output.status.success() || !output_path.exists() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let detail = stderr.trim();
            if detail.is_empty() {
                anyhow::bail!("jj diff did not produce list output");
            }
            anyhow::bail!("jj diff failed: {detail}");
        }

        fs::read_to_string(&output_path)
            .with_context(|| format!("Failed to read list output from {}", output_path.display()))
    })();

    fs::remove_file(&request_path).ok();
    fs::remove_file(&output_path).ok();
    result
}

fn read_diff_summary(revset: Option<&str>) -> Result<Vec<DiffSummaryEntry>> {
    let mut diff_args = vec!["diff", "--template", SUMMARY_TEMPLATE];
    if let Some(rev) = revset {
        diff_args.push("-r");
        diff_args.push(rev);
    }

    let output = Command::new("jj")
        .args(&diff_args)
        .output()
        .context("Failed to run jj diff")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("jj diff failed: {}", stderr.trim());
    }

    let summary = String::from_utf8_lossy(&output.stdout);
    let mut entries = Vec::new();
    for (index, line) in summary.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let entry: DiffSummaryEntry = serde_json::from_str(line)
            .with_context(|| format!("Failed to parse diff summary line {}", index + 1))?;
        entries.push(entry);
    }

    Ok(entries)
}

fn primary_path(entry: &DiffSummaryEntry) -> String {
    if !entry.path.is_empty() {
        entry.path.clone()
    } else if !entry.target.is_empty() {
        entry.target.clone()
    } else {
        entry.source.clone()
    }
}

fn rename_info(entry: &DiffSummaryEntry) -> Option<RenameInfo> {
    match entry.status.as_str() {
        "renamed" | "copied" => {
            if entry.source.is_empty() {
                return None;
            }
            let to = if entry.target.is_empty() {
                entry.path.clone()
            } else {
                entry.target.clone()
            };
            Some(RenameInfo {
                from: entry.source.clone(),
                to,
            })
        }
        _ => None,
    }
}

fn file_paths_for_entry(entry: &DiffSummaryEntry, path: &str) -> FilePaths {
    match entry.status.as_str() {
        "added" => FilePaths {
            before: None,
            after: Some(path.to_string()),
        },
        "removed" => FilePaths {
            before: Some(path.to_string()),
            after: None,
        },
        "renamed" | "copied" => {
            let before = if entry.source.is_empty() {
                path.to_string()
            } else {
                entry.source.clone()
            };
            let after = if entry.target.is_empty() {
                path.to_string()
            } else {
                entry.target.clone()
            };
            FilePaths {
                before: Some(before),
                after: Some(after),
            }
        }
        _ => FilePaths {
            before: Some(path.to_string()),
            after: Some(path.to_string()),
        },
    }
}

fn read_materialized_file(root: &Path, path: Option<&str>) -> Result<Vec<u8>> {
    let Some(path) = path else {
        return Ok(Vec::new());
    };

    let file_path = root.join(path);
    match fs::read(&file_path) {
        Ok(contents) => Ok(contents),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error)
            .with_context(|| format!("Failed to read materialized file {}", file_path.display())),
    }
}

fn is_binary_data(bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return false;
    }

    bytes.contains(&0) || std::str::from_utf8(bytes).is_err()
}

fn truncate_text(
    content: &str,
    max_bytes: Option<usize>,
    max_lines: Option<usize>,
) -> (String, bool) {
    let mut truncated = false;
    let mut result = content.to_string();

    if let Some(max_lines) = max_lines {
        if max_lines == 0 {
            if !result.is_empty() {
                truncated = true;
            }
            result.clear();
        } else {
            let mut limited = String::new();
            let mut count = 0usize;
            for line in result.split_inclusive('\n') {
                if count >= max_lines {
                    truncated = true;
                    break;
                }
                limited.push_str(line);
                count += 1;
            }
            if truncated {
                result = limited;
            }
        }
    }

    if let Some(max_bytes) = max_bytes {
        if result.len() > max_bytes {
            let mut end = max_bytes;
            while !result.is_char_boundary(end) {
                end -= 1;
            }
            result.truncate(end);
            truncated = true;
        }
    }

    (result, truncated)
}

fn spec_decision(spec: Option<&Spec>, path: &str) -> SpecDecision {
    let Some(spec) = spec else {
        return SpecDecision::KeepAll;
    };

    if let Some(file_spec) = spec.files.get(path) {
        match file_spec {
            FileSpec::Action {
                action: Action::Keep,
            } => SpecDecision::KeepAll,
            FileSpec::Action {
                action: Action::Reset,
            } => SpecDecision::Skip,
            FileSpec::Selection(selection) => {
                let selection = selection.to_selection();
                if selection.is_empty() {
                    SpecDecision::Skip
                } else {
                    SpecDecision::KeepSelection(selection)
                }
            }
        }
    } else if spec.default == DefaultAction::Reset {
        SpecDecision::Skip
    } else {
        SpecDecision::KeepAll
    }
}

fn filter_hunks(hunks: Vec<Hunk>, selection: &HunkSelection) -> Vec<Hunk> {
    hunks
        .into_iter()
        .filter(|hunk| selection.matches(hunk.index, &hunk.id))
        .collect()
}

fn normalize_patterns(patterns: &[String]) -> Vec<String> {
    patterns
        .iter()
        .flat_map(|pattern| pattern.split(','))
        .map(|pattern| pattern.trim())
        .filter(|pattern| !pattern.is_empty())
        .map(|pattern| pattern.to_string())
        .collect()
}

fn should_include_entry(entry: &DiffSummaryEntry, include: &[String], exclude: &[String]) -> bool {
    let paths = entry_paths(entry);

    if !include.is_empty() && !paths.iter().any(|path| matches_any(include, path)) {
        return false;
    }

    if !exclude.is_empty() && paths.iter().any(|path| matches_any(exclude, path)) {
        return false;
    }

    true
}

fn entry_paths<'a>(entry: &'a DiffSummaryEntry) -> Vec<&'a str> {
    let mut paths = Vec::new();
    if !entry.path.is_empty() {
        paths.push(entry.path.as_str());
    }
    if !entry.source.is_empty() && entry.source != entry.path {
        paths.push(entry.source.as_str());
    }
    if !entry.target.is_empty() && entry.target != entry.path && entry.target != entry.source {
        paths.push(entry.target.as_str());
    }
    paths
}

fn matches_any(patterns: &[String], path: &str) -> bool {
    patterns.iter().any(|pattern| glob_match(pattern, path))
}

fn glob_match(pattern: &str, path: &str) -> bool {
    let pattern = pattern.trim_start_matches("./");
    let path = path.trim_start_matches("./");

    if pattern.is_empty() {
        return path.is_empty();
    }

    let pattern_segments: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
    let path_segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    match_segments(&pattern_segments, &path_segments)
}

fn match_segments(pattern: &[&str], path: &[&str]) -> bool {
    if pattern.is_empty() {
        return path.is_empty();
    }

    if pattern[0] == "**" {
        if match_segments(&pattern[1..], path) {
            return true;
        }
        if !path.is_empty() {
            return match_segments(pattern, &path[1..]);
        }
        return false;
    }

    if path.is_empty() {
        return false;
    }

    if !match_segment(pattern[0], path[0]) {
        return false;
    }

    match_segments(&pattern[1..], &path[1..])
}

fn match_segment(pattern: &str, text: &str) -> bool {
    if pattern == "*" {
        return true;
    }

    let pattern_chars: Vec<char> = pattern.chars().collect();
    let text_chars: Vec<char> = text.chars().collect();
    let mut dp = vec![vec![false; text_chars.len() + 1]; pattern_chars.len() + 1];

    dp[0][0] = true;
    for i in 1..=pattern_chars.len() {
        if pattern_chars[i - 1] == '*' {
            dp[i][0] = dp[i - 1][0];
        }
    }

    for i in 1..=pattern_chars.len() {
        for j in 1..=text_chars.len() {
            dp[i][j] = match pattern_chars[i - 1] {
                '*' => dp[i - 1][j] || dp[i][j - 1],
                '?' => dp[i - 1][j - 1],
                c => dp[i - 1][j - 1] && c == text_chars[j - 1],
            };
        }
    }

    dp[pattern_chars.len()][text_chars.len()]
}

fn group_files(files: Vec<FileEntry>, grouping: ListGrouping) -> Vec<ListGroup> {
    let mut groups: Vec<ListGroup> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();

    for file in files {
        let key = match grouping {
            ListGrouping::Directory => directory_group(&file.path),
            ListGrouping::Extension => extension_group(&file.path),
            ListGrouping::Status => file.status.clone(),
            ListGrouping::None => String::new(),
        };

        if let Some(position) = index.get(&key).copied() {
            groups[position].files.push(file);
        } else {
            index.insert(key.clone(), groups.len());
            groups.push(ListGroup {
                name: key,
                files: vec![file],
            });
        }
    }

    groups
}

fn build_summary_output(files: Vec<FileEntry>, grouping: ListGrouping) -> ListSummaryOutput {
    let summaries: Vec<FileSummary> = files
        .into_iter()
        .map(|file| FileSummary {
            path: file.path,
            status: file.status,
            rename: file.rename,
            hunk_count: file.hunks.len(),
            binary: file.binary,
            truncated: file.truncated,
        })
        .collect();

    if grouping == ListGrouping::None {
        ListSummaryOutput {
            files: Some(summaries),
            groups: None,
        }
    } else {
        let groups = group_summaries(summaries, grouping);
        ListSummaryOutput {
            files: None,
            groups: Some(groups),
        }
    }
}

fn group_summaries(files: Vec<FileSummary>, grouping: ListGrouping) -> Vec<ListSummaryGroup> {
    let mut groups: Vec<ListSummaryGroup> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();

    for file in files {
        let key = match grouping {
            ListGrouping::Directory => directory_group(&file.path),
            ListGrouping::Extension => extension_group(&file.path),
            ListGrouping::Status => file.status.clone(),
            ListGrouping::None => String::new(),
        };

        if let Some(position) = index.get(&key).copied() {
            groups[position].files.push(file);
        } else {
            index.insert(key.clone(), groups.len());
            groups.push(ListSummaryGroup {
                name: key,
                files: vec![file],
            });
        }
    }

    groups
}

fn build_spec_template(files: Vec<FileEntry>) -> SpecTemplateOutput {
    let mut output = HashMap::new();

    for file in files {
        if file.hunks.is_empty() {
            if file.binary == Some(true) {
                output.insert(
                    file.path,
                    SpecTemplateEntry::Action {
                        action: "keep".to_string(),
                    },
                );
            }
            continue;
        }

        let ids = file.hunks.into_iter().map(|hunk| hunk.id).collect();
        output.insert(file.path, SpecTemplateEntry::Ids { ids });
    }

    SpecTemplateOutput {
        files: output,
        default: "reset".to_string(),
    }
}

fn directory_group(path: &str) -> String {
    let path = Path::new(path);
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_string_lossy().to_string(),
        _ => ".".to_string(),
    }
}

fn extension_group(path: &str) -> String {
    let path = Path::new(path);
    match path.extension() {
        Some(ext) => ext.to_string_lossy().to_string(),
        None => "<no-ext>".to_string(),
    }
}

fn render_text_output(output: &ListOutput) -> String {
    let mut lines = Vec::new();

    if let Some(groups) = &output.groups {
        for (index, group) in groups.iter().enumerate() {
            let name = if group.name == "." || group.name.is_empty() {
                "<root>"
            } else {
                group.name.as_str()
            };
            lines.push(format!("{}:", name));
            format_files_text(&mut lines, &group.files);
            if index + 1 < groups.len() {
                lines.push(String::new());
            }
        }
    } else if let Some(files) = &output.files {
        format_files_text(&mut lines, files);
    }

    if lines.is_empty() {
        return String::new();
    }

    let mut output = lines.join("\n");
    output.push('\n');
    output
}

fn render_text_summary_output(output: &ListSummaryOutput) -> String {
    let mut lines = Vec::new();

    if let Some(groups) = &output.groups {
        for (index, group) in groups.iter().enumerate() {
            let name = if group.name == "." || group.name.is_empty() {
                "<root>"
            } else {
                group.name.as_str()
            };
            lines.push(format!("{}:", name));
            format_summary_text(&mut lines, &group.files);
            if index + 1 < groups.len() {
                lines.push(String::new());
            }
        }
    } else if let Some(files) = &output.files {
        format_summary_text(&mut lines, files);
    }

    if lines.is_empty() {
        return String::new();
    }

    let mut output = lines.join("\n");
    output.push('\n');
    output
}

fn format_files_text(lines: &mut Vec<String>, files: &[FileEntry]) {
    for file in files {
        lines.push(format_file_header(file));
        for unit in &file.file_units {
            lines.push(format!("  {}", file_unit_kind_name(unit.kind)));
            if let Some(removed) = &unit.removed {
                for line in removed.lines() {
                    lines.push(format!("    - {}", line));
                }
            }
            if let Some(added) = &unit.added {
                for line in added.lines() {
                    lines.push(format!("    + {}", line));
                }
            }
        }
        for hunk in &file.hunks {
            lines.push(format!(
                "  hunk {} {} {} (before {}+{} after {}+{})",
                hunk.index,
                hunk.hunk_type,
                hunk.id,
                hunk.before_range.start,
                hunk.before_range.length,
                hunk.after_range.start,
                hunk.after_range.length,
            ));
            if !hunk.removed.is_empty() {
                for line in hunk.removed.lines() {
                    lines.push(format!("    - {}", line));
                }
            }
            if !hunk.added.is_empty() {
                for line in hunk.added.lines() {
                    lines.push(format!("    + {}", line));
                }
            }
        }
    }
}

fn file_unit_kind_name(kind: FileUnitKind) -> &'static str {
    match kind {
        FileUnitKind::Creation => "creation",
        FileUnitKind::Deletion => "deletion",
        FileUnitKind::Rename => "rename",
        FileUnitKind::Mode => "mode",
        FileUnitKind::Binary => "binary",
    }
}

fn format_summary_text(lines: &mut Vec<String>, files: &[FileSummary]) {
    for file in files {
        let mut line = format!(
            "{} {} ({} hunks)",
            status_char(&file.status),
            file.path,
            file.hunk_count
        );
        if let Some(rename) = &file.rename {
            line.push_str(&format!(" ({} -> {})", rename.from, rename.to));
        }
        if file.binary == Some(true) {
            line.push_str(" [binary]");
        }
        if file.truncated == Some(true) {
            line.push_str(" [truncated]");
        }
        lines.push(line);
    }
}

fn format_file_header(file: &FileEntry) -> String {
    let mut header = format!("{} {}", status_char(&file.status), file.path);
    if let Some(rename) = &file.rename {
        header.push_str(&format!(" ({} -> {})", rename.from, rename.to));
    }
    if file.binary == Some(true) {
        header.push_str(" [binary]");
    }
    if file.truncated == Some(true) {
        header.push_str(" [truncated]");
    }
    header
}

fn status_char(status: &str) -> &'static str {
    match status {
        "modified" => "M",
        "added" => "A",
        "removed" => "D",
        "renamed" => "R",
        "copied" => "C",
        _ => "?",
    }
}

/// Select hunks (called by jj --tool)
pub fn select(left: &str, right: &str) -> Result<()> {
    let spec_path = std::env::var("JJ_HUNK_SELECTION").ok();

    let spec = if let Some(path) = spec_path {
        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read spec from {}", path))?;
        Spec::from_str(&content)?
    } else {
        // No selection = keep everything
        return Ok(());
    };

    let left_path = Path::new(left);
    let right_path = Path::new(right);

    // Get all files in both directories
    let left_files = list_files(left_path);
    let right_files = list_files(right_path);
    let all_files: HashSet<_> = left_files.union(&right_files).cloned().collect();

    for filepath in all_files {
        let file_spec = spec.files.get(&filepath);

        match file_spec {
            Some(FileSpec::Action {
                action: Action::Keep,
            }) => {
                // Keep as-is
            }
            Some(FileSpec::Action {
                action: Action::Reset,
            }) => {
                reset_file(left_path, right_path, &filepath)?;
            }
            Some(FileSpec::Selection(selection)) => {
                let selection = selection.to_selection();
                apply_hunk_selection(left_path, right_path, &filepath, &selection)?;
            }
            None => {
                // Use default
                if spec.default == DefaultAction::Reset {
                    reset_file(left_path, right_path, &filepath)?;
                }
            }
        }
    }

    Ok(())
}

fn list_files(dir: &Path) -> HashSet<String> {
    let mut files = HashSet::new();
    if !dir.exists() {
        return files;
    }

    for entry in WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
        if entry.file_type().is_file() {
            if let Ok(rel) = entry.path().strip_prefix(dir) {
                let name = rel.to_string_lossy().to_string();
                if name != "JJ-INSTRUCTIONS" {
                    files.insert(name);
                }
            }
        }
    }
    files
}

fn reset_file(left: &Path, right: &Path, filepath: &str) -> Result<()> {
    let left_file = left.join(filepath);
    let right_file = right.join(filepath);

    if left_file.exists() {
        if let Some(parent) = right_file.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&left_file, &right_file)?;
    } else if right_file.exists() {
        fs::remove_file(&right_file)?;
    }
    Ok(())
}

fn apply_hunk_selection(
    left: &Path,
    right: &Path,
    filepath: &str,
    selection: &HunkSelection,
) -> Result<()> {
    let left_file = left.join(filepath);
    let right_file = right.join(filepath);

    let before = if left_file.exists() {
        fs::read_to_string(&left_file)?
    } else {
        String::new()
    };

    let after = if right_file.exists() {
        fs::read_to_string(&right_file)?
    } else {
        return Ok(());
    };

    let result = apply_selected_hunks(&before, &after, selection);

    fs::write(&right_file, result)?;
    Ok(())
}

fn resolve_spec_input(spec: Option<&str>, spec_file: Option<&str>) -> Result<String> {
    if let Some(path) = spec_file {
        if path.is_empty() {
            anyhow::bail!("Spec file path is empty");
        }
        return fs::read_to_string(path)
            .with_context(|| format!("Failed to read spec file {}", path));
    }

    let spec = spec.ok_or_else(|| anyhow::anyhow!("Spec is required (or use --spec-file)"))?;
    if spec == "-" {
        let mut buffer = String::new();
        std::io::stdin()
            .read_to_string(&mut buffer)
            .context("Failed to read spec from stdin")?;
        if buffer.trim().is_empty() {
            anyhow::bail!("Spec from stdin is empty");
        }
        return Ok(buffer);
    }

    Ok(spec.to_string())
}

fn run_jj_with_selection(args: &[&str], spec: Option<&str>, spec_file: Option<&str>) -> Result<()> {
    let spec_content = resolve_spec_input(spec, spec_file)?;
    let temp_file = std::env::temp_dir().join(format!("jj-hunk-{}.spec", std::process::id()));
    fs::write(&temp_file, spec_content)?;

    let config_args = jj_hunk_tool_config_args()?;

    let status = Command::new("jj")
        .args(&config_args)
        .args(args)
        .env("JJ_HUNK_SELECTION", &temp_file)
        .status()
        .context("Failed to run jj")?;

    fs::remove_file(&temp_file).ok();

    if !status.success() {
        anyhow::bail!("jj command failed");
    }
    Ok(())
}

fn jj_hunk_tool_config_args() -> Result<Vec<String>> {
    let mut args = Vec::new();

    if !jj_config_key_exists(JJ_HUNK_PROGRAM_KEY) {
        let program = std::env::current_exe()
            .context("Failed to determine current jj-hunk executable path")?;
        let program = program
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("jj-hunk executable path is not valid UTF-8"))?;
        args.push("--config".to_string());
        args.push(format!("{JJ_HUNK_PROGRAM_KEY}={}", toml_string(program)));
    }

    if !jj_config_key_exists(JJ_HUNK_EDIT_ARGS_KEY) {
        args.push("--config".to_string());
        args.push(format!(
            r#"{JJ_HUNK_EDIT_ARGS_KEY}=["select", "$left", "$right"]"#
        ));
    }

    Ok(args)
}

fn jj_config_key_exists(key: &str) -> bool {
    Command::new("jj")
        .args(["config", "get", key])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn toml_string(value: &str) -> String {
    serde_json::to_string(value).expect("string serialization should not fail")
}

pub fn split(
    spec: Option<&str>,
    spec_file: Option<&str>,
    message: &str,
    rev: Option<&str>,
) -> Result<()> {
    let mut args = vec!["split", JJ_HUNK_TOOL_ARG, "-m", message];
    if let Some(rev) = rev {
        args.push("-r");
        args.push(rev);
    }
    run_jj_with_selection(&args, spec, spec_file)
}

pub fn commit(spec: Option<&str>, spec_file: Option<&str>, message: &str) -> Result<()> {
    run_jj_with_selection(
        &["commit", "-i", JJ_HUNK_TOOL_ARG, "-m", message],
        spec,
        spec_file,
    )
}

pub fn squash(spec: Option<&str>, spec_file: Option<&str>, rev: Option<&str>) -> Result<()> {
    let mut args = vec!["squash", "-i", JJ_HUNK_TOOL_ARG];
    if let Some(rev) = rev {
        args.push("-r");
        args.push(rev);
    }
    run_jj_with_selection(&args, spec, spec_file)
}
