# Hunkset language: decisions and implementation

Status: implementation record. The original six stages shipped in 0.5.0.
The 0.5.1 interface uses operation-first predicates and named pattern arguments.
Parked decisions remain out of scope until a later feature needs them.

## Working decisions

- Use a declarative set language. Supply the comparison scope separately through
  the caller, rather than embedding revsets in the core expression.
- Distinguish changes contributed by individual revisions from the net difference
  between two states. Do not silently treat them as equivalent.
- Select whole small text blocks. An unchanged line separates blocks. Multiple
  blocks can compose into larger selections without becoming one selection unit.
- Include indivisible file changes. Rename and text changes are separate units.
  File creation includes initial content; deletion includes removed content.
  Empty files still require a unit. Include binary and mode changes.
- Content matching is a case-sensitive literal substring by default, with an
  explicit regex form. Search added and removed text by default; support explicit
  sides. Exclude unchanged context. A match selects the whole block.
- Multiple conditions must match the same unit, but can match different lines or
  different sides. Multiline matches can span consecutive lines on one side,
  never across removed and added text.
- Path arguments match either old or new path by default, with explicit
  before_path/after_path fields; renamed() also supports from/to. Evaluate each
  pattern expression independently against each available path or text side.
- Operation-first functions are changed, added, removed, renamed, mode_changed,
  and binary_changed, plus all/none/id. No kind argument is needed: path means
  location; file on added/removed restricts selection to whole-file operations.
  Content on added/removed searches that text side, including initial/deleted
  text of file units. A content match never splits an indivisible unit.
- Pattern modifiers substring/exact/glob/regex describe matching independently
  from the field. Content defaults to substring; paths default to glob. Reject
  unknown, duplicate, or operation-incompatible fields before evaluation.
- Pure rename, mode, and binary units do not match text predicates. Complement
  can therefore retain them.
- Support union, intersection, difference, complement, and parentheses.
  Complement binds strongest; intersection and difference share precedence and
  associate left; union binds weakest. Complement is local to the supplied scope.
- Valid queries with no matches return an empty set. Invalid syntax and unknown
  functions are errors.
- Sets contain distinct occurrences, not deduplicated text. Identical edits in
  different files, comparisons, or positions remain distinct. Sets are unordered;
  the caller controls display order.
- Support named aliases and set-expression parameters through an environment
  explicitly supplied by the caller.
- `jj-hunk` loads alias definitions from the effective `[hunkset-aliases]`
  configuration and lets command-line definitions replace configured aliases by
  name. The `hunkset` library remains independent of configuration and I/O.
- Selecting a rename does not select text changes in its file, or the reverse.

Example:

```text
renamed(path:glob:"src/**")
|
changed(path:glob:"src/**" | glob:"tests/**", content:"timeout")
```

This selects all renames under src, plus timeout text blocks under src or tests.
It does not restrict renames to files with matching timeout text.

## Parked questions

Revisit each question when the listed feature requires it. These are not all
prerequisites for a first query preview.

| Question | Why it matters | Revisit when |
|---|---|---|
| Does the language mandate comparison rules, or accept caller-defined rules? | Rename detection, text alignment, and normalization can change the selectable units. Proposed starting point: caller supplies units. | Supporting a second change producer or reproducible saved requests |
| What comparison does a merge contribute? | Comparing against either parent or their automatic combination selects different changes. | Generalizing revision scope; keep this outside the evaluator |
| What is saved: query, fixed scope, moving scope, or selected IDs? | Re-evaluation and exact reference lookup have different behavior. | Saving requests or selections |
| Do selections follow edits across rewritten comparisons? | Exact identity does not establish correspondence across rewrites. | An explicit tracking feature; not required for basic IDs |
| Which line-range and ID selectors are useful? | Location selection and exact occurrence selection answer different requests. Pijul suggests separating visible handles from stored content identity. | Manual selection after preview |
| How do semantic selectors and relations compose? | For example, select edits inside a function, or renames of files with matching edits. | After the core language works on real changes |

Pijul references: [recording sections](https://pijul.org/manual/workflows/splitting_and_combining_changes)
and [content identity](https://pijul.org/manual/theory). These are inspiration,
not a requirement to adopt its storage model or a claim that it has our proposed DSL.

## Implemented boundary

Use one Cargo workspace in this repository, retaining the root jj-hunk package
and adding a local library at crates/hunkset. Do not create a separate repository
or publish the crate yet.

- hunkset: query parser, diagnostics, selectable-unit input contract, predicates,
  and set evaluation. Accept prepared units and return selected occurrence keys.
  No subprocesses, repository access, revset resolution, or file writes.
- jj-hunk: resolve scope, obtain before/after data, construct units, render
  results, and execute split/commit/squash operations.
- Keep parser and evaluator in one crate. Do not build an extension framework
  or add a Git dependency to the evaluator for the first version.
- Use evaluation-local occurrence keys internally. Public occurrence IDs are
  exact, comparison-bound SHA-256 values over a versioned canonical encoding.
  They identify text and file units in one complete materialized comparison;
  they are not persistent identities across rewritten comparisons.

## Delivery slices

1. Add the library and test realistic fixtures through its public interface.
   Implement all/none, file and literal-content predicates, set operators,
   parentheses, and errors. Test duplicate-looking occurrences and non-text units.
2. Add a read-only jj-hunk list query option, with revision scope separate.
   Adapt current text blocks first. Add file-level units explicitly; do not claim
   rename support merely because the existing output has rename metadata.
   Evaluate complete content, not a display-truncated diff. Make any unavailable
   input capability explicit rather than returning misleading results.
3. Add mutation integration after preview and execution can use the same unit
   construction and selection semantics. Test rename-only, text-only in a renamed
   file, their combination, new/deleted files, binary changes, and mode changes.
4. Added explicit side selectors and regex in stage 5. Named and parameterized
   aliases are implemented and accepted in stage 6. Revisit parked
   decisions only as later features require them.
5. Added exact `id()` lookup and comparison-bound occurrence IDs for text and
   file units. The legacy spec format continues to use text IDs and whole-file
   actions; file-unit IDs are available through query output and `id()`.

Current user-visible query form:

```sh
jj-hunk list -r @ --query 'changed(path:glob:"src/**", content:"timeout")'
```

## Pre-implementation observations (historical)

- src/diff.rs get_hunks already ends a text block at an unchanged line.
- The former compute_hunk_id hashed kind, changed text, and nearby context, not
  paths or comparison identity. The implemented occurrence encoding also uses
  the complete comparison, paths, bytes, modes, unit kind, and text spans.
- src/commands.rs render_materialized_list can omit non-binary entries without
  text hunks. Rename metadata is not a separate selectable rename unit.
- Listing can truncate content before diffing. Query evaluation must not silently
  use that truncated content as the complete selection universe.
- The mutation path uses per-file specs. Independent rename/text selection needs
  explicit integration work; it is not just a parser change.
