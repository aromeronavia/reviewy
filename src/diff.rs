/// The role of one line inside a unified diff hunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffLineKind {
    Hunk,
    Added,
    Removed,
    Context,
}

/// One line of a patch, with its leading marker kept so columns line up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub text: String,
}

const TAB_WIDTH: usize = 4;

/// Splits a per-file patch (as returned by the GitHub API) into typed lines.
pub fn parse_patch(patch: &str) -> Vec<DiffLine> {
    patch
        .lines()
        .map(|line| DiffLine {
            kind: classify(line),
            text: line.replace('\t', &" ".repeat(TAB_WIDTH)),
        })
        .collect()
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
}
