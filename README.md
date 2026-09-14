# jj-hunk

Programmatic hunk selection for [jj (Jujutsu)](https://github.com/martinvonz/jj).

Select specific diff hunks when splitting, committing, or squashing—without interactive UI. Designed for AI agents and automation.

## Installation

### 1. Install the binary

```bash
cargo install jj-hunk
```

### 2. Verify

```bash
jj-hunk --help
```

## Quick Start

```bash
# See what hunks exist in your changes
jj-hunk list

# See hunks for a specific revision (diff vs parent)
# Note: revset must resolve to a single revision
jj-hunk list --rev @

# Emit YAML instead of JSON
jj-hunk list --format yaml

# Select whole text blocks with a query; revision scope stays separate
jj-hunk list -r @ --query 'glob("src/**") & content("timeout")'

# Apply the same query selection while the other changes stay in the working copy
jj-hunk commit --query 'glob("src/fix.rs")' "bug fix"

# List files only (hunk counts)
jj-hunk list --files

# Emit a spec template using stable ids
jj-hunk list --spec-template --format yaml

# Split changes: hunks 0,1 of foo.rs → first commit, rest → second
jj-hunk split '{"files": {"src/foo.rs": {"hunks": [0, 1]}}, "default": "reset"}' "first commit"

# Split a specific revision (not just working copy)
jj-hunk split -r @- '{"files": {"src/foo.rs": {"action": "keep"}}, "default": "reset"}' "first commit"

# Commit specific files, leave rest in working copy
jj-hunk commit '{"files": {"src/fix.rs": {"action": "keep"}}, "default": "reset"}' "bug fix"

# Squash specific changes into parent
jj-hunk squash '{"files": {"src/cleanup.rs": {"action": "keep"}}, "default": "reset"}'

# Squash a specific revision into its parent
jj-hunk squash -r @- '{"files": {"src/cleanup.rs": {"action": "keep"}}, "default": "reset"}'

# Read spec from a file (JSON or YAML)
jj-hunk split --spec-file spec.yaml "first commit"

# Read spec from stdin
cat spec.json | jj-hunk commit - "bug fix"
```

## Commands

| Command | Description |
|---------|-------------|
| `jj-hunk list [options]` | List hunks, files, or spec templates |
| `jj-hunk split [-r rev] <spec> <message>` | Split changes into two commits |
| `jj-hunk split [-r rev] --query <expression> <message>` | Split query-selected occurrences |
| `jj-hunk commit <spec> <message>` | Commit selected hunks |
| `jj-hunk commit --query <expression> <message>` | Commit query-selected occurrences |
| `jj-hunk squash [-r rev] <spec>` | Squash selected hunks into parent |
| `jj-hunk squash [-r rev] --query <expression>` | Squash query-selected occurrences |

Split and squash accept `-r <rev>` to target any revision (default: `@`). Commit always operates on the working copy.

List options:
- `--rev <revset>` — diff the revision against its parent (revset must resolve to a single revision)
- `--format json|yaml|text` — output format (default: json)
- `--include <glob>` / `--exclude <glob>` — filter paths (repeatable, supports `**`, `*`, `?`)
- `--group none|directory|extension|status` — group output
- `--binary skip|mark|include` — binary handling (default: mark)
- `--max-bytes <n>` / `--max-lines <n>` — limit displayed changed text after IDs and queries use the complete diff
- `--spec <json|yaml>` / `--spec-file <path>` — preview using a spec filter
- `--query <expression>` — select occurrences with text, path, file-kind, and set predicates described below
- `--files` — list files with hunk counts only
- `--spec-template` — emit a spec template (JSON/YAML only)

`<spec>` may be an inline JSON/YAML string or `-` to read from stdin. Use `--spec-file <path>` to read a JSON/YAML file (omit `<spec>` when using `--spec-file`).

Mutation commands accept `--query` as an alternative to a spec. Query evaluation uses the complete materialized diff. A valid query that selects no occurrences is a no-op for split, commit, and squash; it does not select all changes. Query and spec inputs cannot be combined.

### Query predicates

- `content("text")` searches either changed text side. `added_text("text")` and `removed_text("text")` search only that side.
- `regex("pattern")` searches either changed text side. `added_regex("pattern")` and `removed_regex("pattern")` search only that side. Regex syntax is the Rust `regex` syntax; inline flags such as `(?i)` are explicit. Escape a regex backslash in the query string, for example `regex("timeout\\s*=\\s*\\d+")`.
- `glob(<glob-expression>)` matches either path. `before_glob(<glob-expression>)` and `after_glob(<glob-expression>)` match only that path. A glob expression supports quoted glob patterns, `|`, `&`, binary `~`, unary `~`, and parentheses. This is a small hunkset grammar; it does not accept the full `jj` fileset language or its functions.
- `added()`, `deleted()`, `renames()`, `modes()`, and `binaries()` select indivisible file changes. `all()` and `none()` select the full or empty occurrence set.
- `id("hunk-<64 hex characters>")` selects one exact occurrence ID from list output. Prefix matching is not supported.

Literal and regex matching is case-sensitive by default. Text predicates search changed text only, not unchanged context. A multiline pattern can span consecutive lines within one removed or added side. It cannot cross from removed text to added text. Hunkset expressions compose with `|`, `&`, binary `~`, unary `~`, and parentheses.

### Query aliases

Use repeatable global `--alias 'name(parameters)=expression'` options. Parameters are set expressions and are referenced with `parameter()` in the alias body. For example:

```bash
jj-hunk \
  --alias 'generated()=glob("generated/**")' \
  --alias 'handwritten(selection)=(all() ~ generated()) & selection()' \
  list --query 'handwritten(content("timeout"))'
```

The same options work with `split`, `commit`, and `squash`. Alias arguments are parsed expressions, so substitution preserves parentheses and operator precedence. Alias definition syntax, names, builtin collisions, duplicate names and parameters, and body syntax are validated when the environment is built. Wrong arity, unknown names, cycles, more than 32 nested expansions, and more than 10,000 expanded expression or glob-expression nodes are checked when a query references the alias. All checks for the requested query finish before a mutation starts; unused alias bodies are not recursively resolved.

Aliases can also be stored in effective `jj` configuration. Quote each signature because TOML bare keys cannot contain parentheses:

```toml
[hunkset-aliases]
"generated()" = 'glob("generated/**")'
"handwritten(selection)" = '(all() ~ generated()) & selection()'
```

`jj-hunk` reads the effective values through `jj config`, so normal user, repository, and workspace precedence applies. A `--alias` definition replaces a configured definition with the same alias name, including its parameter signature. Duplicate configured names and duplicate command-line names remain errors because alias overloading is not supported.

The `hunkset` library does not read CLI or repository configuration. `jj-hunk` loads configuration and supplies an explicit environment. Other callers construct aliases with `AliasDefinition::new`, collect them with `AliasEnvironment::new`, and call `evaluate_with_aliases`. `evaluate` uses an empty alias environment.

## Spec Format

Specs can be **JSON or YAML**. Inline JSON is convenient for short specs; use `--spec-file` or stdin for larger ones. You can select text hunks by index (`hunks`) or by exact `ids` emitted by `jj-hunk list`. IDs use the form `hunk-<64 lowercase hexadecimal characters>`. They bind an occurrence to the complete materialized before/after comparison, paths, file bytes, executable state, and text location. The same comparison produces the same IDs, independent of display limits. Any comparison change can regenerate all IDs, so do not reuse a saved ID after a revision or working-copy change. Releases before this contract used content/context hashes; regenerate saved specs because there is no legacy-ID fallback. Occurrence IDs currently require a Unix platform so executable state is part of the comparison. `hunks` entries may also be ID strings.

```json
{
  "files": {
    "path/to/file": {"hunks": [0, "hunk-7c3d...", 2]},
    "path/to/other": {"ids": ["hunk-9a2b..."]},
    "path/to/another": {"action": "keep"},
    "path/to/skip": {"action": "reset"}
  },
  "default": "reset"
}
```

- `{"hunks": [indices|ids]}` — select by index (0-based) or id string
- `{"ids": ["hunk-..."]}` — select hunks by id from `jj-hunk list`
- `{"action": "keep"}` — keep all changes in file
- `{"action": "reset"}` — discard all changes in file
- `"default"` — action for unlisted files (`"keep"` or `"reset"`)

`ids` and `hunks` are merged if both are provided. Use `jj-hunk list --spec-template` to generate an id-based starting spec.

Query preview returns changed text blocks in `hunks` and selected indivisible changes in `file_units`. Both forms include exact occurrence IDs for `id()` queries. Creation and deletion units include their complete added or removed text, including an empty string for empty files. Rename, mode, and binary units are separate from text blocks in the same file. The legacy spec schema keeps text-hunk IDs and whole-file keep/reset actions. Its single-hunk projection for a nonempty creation or deletion shares the corresponding file-unit ID. Legacy hunk or ID selection rejects renamed files; use a query with `id()` for those occurrences. Rename, mode, binary, and empty-file unit IDs are query-only. Empty-file and other file-only occurrences are available with `list --query 'all()'`. Copies, conflicts, symlinks, trees, and submodules report explicit unsupported-input errors. `--query` cannot be combined with `--spec`, `--spec-file`, `--include`, `--exclude`, `--files`, or `--spec-template`. `--max-bytes` and `--max-lines` limit displayed selected text after the query evaluates complete content.

## Example Output

```bash
$ jj-hunk list --format json
{
  "files": [
    {
      "path": "src/lib.rs",
      "status": "modified",
      "hunks": [
        {
          "id": "hunk-4c1b1b3...",
          "index": 0,
          "type": "replace",
          "removed": "old_fn()\n",
          "added": "new_fn()\n",
          "before": {"start": 10, "lines": 1},
          "after": {"start": 10, "lines": 1},
          "context": {"pre": "// prev\n", "post": "// next\n"}
        }
      ]
    },
    {
      "path": "src/main.rs",
      "status": "deleted",
      "hunks": [
        {
          "id": "hunk-771ad9f...",
          "index": 0,
          "type": "delete",
          "removed": "dead_code()\n",
          "added": "",
          "before": {"start": 1, "lines": 1},
          "after": {"start": 1, "lines": 0}
        }
      ]
    }
  ]
}
```

- `files` is a list of file entries. Each entry includes `status`, optional `rename`, and `hunks`.
- Each hunk includes a stable `id` (sha256), `index`, line ranges (`before`/`after`), and optional `context`.
- When grouped (`--group`), output uses `groups: [{name, files}]` instead of `files`.

### List Modes

```bash
# Files-only summary
jj-hunk list --files --format text

# Spec template (ids, default reset)
jj-hunk list --spec-template --format yaml
```

### Filtering and Grouping

```bash
jj-hunk list --include 'src/**' --exclude '**/*.test.rs' --group directory
```

## How It Works

jj-hunk integrates with jj's `--tool` mechanism:

1. You run `jj-hunk split/commit/squash` with a JSON/YAML spec
2. jj-hunk writes the spec to a temp file and sets `JJ_HUNK_SELECTION` env var
3. jj-hunk passes temporary `merge-tools.jj-hunk` config to jj when needed
4. jj invokes `jj-hunk select $left $right` as the diff tool
5. jj-hunk reads the spec and modifies `$right` to include only selected hunks
6. jj snapshots the result

For direct control with `jj` itself, provide the same tool config explicitly or define it in your jj config:

```bash
echo '{"files": {"src/foo.rs": {"hunks": [0]}}}' > /tmp/spec.json
JJ_HUNK_SELECTION=/tmp/spec.json jj \
  --config 'merge-tools.jj-hunk.program="jj-hunk"' \
  --config 'merge-tools.jj-hunk.edit-args=["select", "$left", "$right"]' \
  split -i --tool=jj-hunk -m "message"
```

## Use Cases

### AI Agents

The primary use case. AI agents can create clean, logical commits without interactive prompts. Instead of dumping all changes into one commit, an agent can:

1. Analyze changes with `jj-hunk list`
2. Group files by logical concern (schema, services, tests, etc.)
3. Split iteratively to create a narrative commit history

The JSON/YAML spec format is easy for LLMs to construct programmatically.

### Clean History Workflow

Reorganize messy development history into reviewer-friendly commits. Squash everything, then split by concern:

```bash
jj squash --from 'all:trunk()..@-' --into @
jj edit @
jj-hunk split '{"files": {"src/db/schema.ts": {"action": "keep"}}, "default": "reset"}' "feat: add schema"
jj-hunk split '{"files": {"src/api/routes.ts": {"action": "keep"}}, "default": "reset"}' "feat: add routes"
jj describe -m "feat: add UI"
```

See `.claude/commands/clean-history.md` for a complete workflow.

### CI/CD Automation

Script commit splitting in pipelines. Enforce commit hygiene rules, auto-split by file patterns, or validate that commits are properly scoped.

### Partial Commits

Keep experimental code in working copy while committing only the finished parts:

```bash
jj-hunk commit '{"files": {"src/fix.rs": {"action": "keep"}}, "default": "reset"}' "fix: handle edge case"
# Experimental changes remain uncommitted
```

## License

MIT
