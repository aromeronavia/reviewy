/// The role of one line inside a unified diff hunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffLineKind {
    Hunk,
    Added,
    Removed,
    Context,
}

/// One line of a patch, with its leading marker kept so columns line up.
/// `old_line` is the line's number in the base file, `new_line` in the head
/// file; hunk headers and `\` marker lines have neither.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub text: String,
    pub old_line: Option<u64>,
    pub new_line: Option<u64>,
}

const TAB_WIDTH: usize = 4;

/// Splits a per-file patch (as returned by the GitHub API) into typed lines.
pub fn parse_patch(patch: &str) -> Vec<DiffLine> {
    let (mut old, mut new) = (0u64, 0u64);
    patch
        .lines()
        .map(|line| {
            let kind = classify(line);
            let text = line.replace('\t', &" ".repeat(TAB_WIDTH));
            let (old_line, new_line) = match kind {
                DiffLineKind::Hunk => {
                    if let Some((start_old, start_new)) = hunk_start(line) {
                        old = start_old;
                        new = start_new;
                    }
                    (None, None)
                }
                _ if line.starts_with('\\') => (None, None),
                DiffLineKind::Added => {
                    new += 1;
                    (None, Some(new - 1))
                }
                DiffLineKind::Removed => {
                    old += 1;
                    (Some(old - 1), None)
                }
                DiffLineKind::Context => {
                    old += 1;
                    new += 1;
                    (Some(old - 1), Some(new - 1))
                }
            };
            DiffLine { kind, text, old_line, new_line }
        })
        .collect()
}

/// Reads the starting line numbers out of `@@ -a[,b] +c[,d] @@`.
fn hunk_start(header: &str) -> Option<(u64, u64)> {
    let mut parts = header.split_whitespace();
    let old = parts.nth(1)?.strip_prefix('-')?;
    let new = parts.next()?.strip_prefix('+')?;
    let first_number = |range: &str| range.split(',').next()?.parse::<u64>().ok();
    Some((first_number(old)?, first_number(new)?))
}

fn classify(line: &str) -> DiffLineKind {
    if line.starts_with("@@") {
        DiffLineKind::Hunk
    } else if line.starts_with('+') {
        DiffLineKind::Added
    } else if line.starts_with('-') {
        DiffLineKind::Removed
    } else {
        DiffLineKind::Context
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_each_line_kind() {
        let lines = parse_patch("@@ -1,2 +1,2 @@\n ctx\n-old\n+new\n\\ No newline at end of file");
        let kinds: Vec<_> = lines.iter().map(|l| l.kind).collect();
        assert_eq!(
            kinds,
            [
                DiffLineKind::Hunk,
                DiffLineKind::Context,
                DiffLineKind::Removed,
                DiffLineKind::Added,
                DiffLineKind::Context,
            ]
        );
        assert_eq!(lines[2].text, "-old");
    }

    #[test]
    fn expands_tabs_to_four_spaces() {
        assert_eq!(parse_patch("+\tx")[0].text, "+    x");
    }

    #[test]
    fn empty_patch_gives_no_lines() {
        assert!(parse_patch("").is_empty());
    }

    #[test]
    fn numbers_context_added_and_removed_lines() {
        let lines = parse_patch("@@ -10,3 +20,3 @@\n a\n-b\n+c\n d");
        let numbers: Vec<_> = lines.iter().map(|l| (l.old_line, l.new_line)).collect();
        assert_eq!(
            numbers,
            [
                (None, None),
                (Some(10), Some(20)),
                (Some(11), None),
                (None, Some(21)),
                (Some(12), Some(22)),
            ]
        );
    }

    #[test]
    fn second_hunk_restarts_numbering() {
        let lines = parse_patch("@@ -1,2 +1,2 @@\n a\n b\n@@ -50 +60 @@\n c");
        assert_eq!((lines[4].old_line, lines[4].new_line), (Some(50), Some(60)));
    }

    #[test]
    fn hunk_and_no_newline_marker_have_no_numbers() {
        let lines = parse_patch("@@ -1 +1 @@\n-x\n\\ No newline at end of file\n+y");
        assert_eq!((lines[0].old_line, lines[0].new_line), (None, None));
        assert_eq!((lines[2].old_line, lines[2].new_line), (None, None));
        assert_eq!((lines[3].old_line, lines[3].new_line), (None, Some(1)));
    }

    #[test]
    fn hunk_without_counts_parses() {
        let lines = parse_patch("@@ -1 +1 @@\n-x\n+y");
        assert_eq!(lines[1].old_line, Some(1));
        assert_eq!(lines[2].new_line, Some(1));
    }
}
