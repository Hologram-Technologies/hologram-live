//! `apps/registry/DIFFERENCES.md` is the allowlist. A difference that is not
//! listed fails the gate. So does a listed difference that no longer happens:
//! the file operators read must never claim a difference that is gone.

use crate::compare::Difference;

const BEGIN: &str = "<!-- gate-b:begin -->";
const END: &str = "<!-- gate-b:end -->";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub id: String,
    pub scenario: String,
    /// `*` matches every step of the scenario.
    pub step: String,
    /// `*` matches every field of the step: the whole answer differs by design.
    pub field: String,
    pub reference: String,
    pub product: String,
}

#[derive(Debug, Default)]
pub struct Verdict {
    pub unlisted: Vec<Difference>,
    pub listed: Vec<(String, Difference)>,
    pub stale: Vec<String>,
}

impl Verdict {
    pub fn passed(&self) -> bool {
        self.unlisted.is_empty() && self.stale.is_empty()
    }
}

/// The rows of the table between the two markers.
pub fn parse(markdown: &str) -> Result<Vec<Row>, String> {
    let Some(start) = markdown.find(BEGIN) else {
        return Ok(Vec::new());
    };
    let table = &markdown[start + BEGIN.len()..];
    let table = &table[..table.find(END).ok_or("gate-b:begin without gate-b:end")?];
    let mut rows = Vec::new();
    for line in table.lines().map(str::trim).filter(|line| line.starts_with('|')) {
        let cells: Vec<String> = split_cells(line);
        if cells.first().is_some_and(|id| id == "id" || id.starts_with("---")) {
            continue;
        }
        let [id, scenario, step, field, reference, product, _reason] = <[String; 7]>::try_from(cells)
            .map_err(|cells| format!("an allowlist row needs 7 cells, found {}: {line}", cells.len()))?;
        rows.push(Row { id, scenario, step, field, reference, product });
    }
    Ok(rows)
}

/// Split on `|`, keeping `\|` as a literal pipe inside a cell.
fn split_cells(line: &str) -> Vec<String> {
    let mut cells = vec![String::new()];
    let mut chars = line.trim().trim_start_matches('|').chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'|') => {
                chars.next();
                cells.last_mut().expect("one cell").push_str("\\|");
            }
            '|' => cells.push(String::new()),
            other => cells.last_mut().expect("one cell").push(other),
        }
    }
    if cells.last().is_some_and(|last| last.trim().is_empty()) {
        cells.pop();
    }
    cells.into_iter().map(|cell| cell.trim().to_owned()).collect()
}

/// Only the scenarios that ran can make a row stale.
pub fn judge(differences: &[Difference], rows: &[Row], ran: &[String]) -> Verdict {
    let mut verdict = Verdict::default();
    let mut used = vec![false; rows.len()];
    for difference in differences {
        // `*` in step, field, reference or product matches anything there.
        let matches = |pattern: &str, value: &str| pattern == "*" || pattern == value;
        let hit = rows.iter().position(|row| {
            row.scenario == difference.scenario
                && matches(&row.step, &difference.step)
                && matches(&row.field, &difference.field)
                && matches(&row.reference, &difference.left)
                && matches(&row.product, &difference.right)
        });
        match hit {
            Some(at) => {
                used[at] = true;
                verdict.listed.push((rows[at].id.clone(), difference.clone()));
            }
            None => verdict.unlisted.push(difference.clone()),
        }
    }
    for (row, used) in rows.iter().zip(used) {
        if !used && ran.contains(&row.scenario) {
            verdict.stale.push(row.id.clone());
        }
    }
    verdict
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diff(scenario: &str, step: &str, field: &str, left: &str, right: &str) -> Difference {
        Difference { scenario: scenario.to_owned(), step: step.to_owned(), field: field.to_owned(), left: left.to_owned(), right: right.to_owned() }
    }

    fn allowlist(rows: &str) -> Vec<Row> {
        parse(&format!("prose\n{BEGIN}\n| id | scenario | step | field | reference | product | reason |\n|---|---|---|---|---|---|---|\n{rows}\n{END}\n")).expect("allowlist")
    }

    fn ran() -> Vec<String> {
        vec!["base".to_owned()]
    }

    #[test]
    fn an_unlisted_difference_fails() {
        let verdict = judge(&[diff("base", "get", "status", "200", "204")], &allowlist(""), &ran());
        assert_eq!(verdict.unlisted.len(), 1);
        assert!(!verdict.passed());
    }

    #[test]
    fn a_listed_difference_passes() {
        let list = allowlist("| D-001 | base | get | status | 200 | 204 | because |");
        assert!(judge(&[diff("base", "get", "status", "200", "204")], &list, &ran()).passed());
    }

    #[test]
    fn a_listed_difference_that_no_longer_happens_fails_as_stale() {
        let list = allowlist("| D-001 | base | get | status | 200 | 204 | because |");
        let verdict = judge(&[], &list, &ran());
        assert_eq!(verdict.stale, ["D-001"]);
        assert!(!verdict.passed(), "the file must never claim a difference that is gone");
    }

    #[test]
    fn a_row_for_a_scenario_that_did_not_run_is_not_stale() {
        let list = allowlist("| D-001 | mount | hit | status | 201 | 202 | because |");
        assert!(judge(&[], &list, &ran()).passed());
    }

    #[test]
    fn a_star_covers_every_step_and_the_values_must_still_match() {
        let list = allowlist("| D-002 | base | * | header:x-content-type-options | nosniff | (absent) | because |");
        let same = diff("base", "anything", "header:x-content-type-options", "nosniff", "(absent)");
        let other = diff("base", "anything", "header:x-content-type-options", "nosniff", "sniff");
        assert!(judge(&[same], &list, &ran()).passed());
        assert!(!judge(&[other], &list, &ran()).passed());
    }

    #[test]
    fn a_whole_step_can_be_listed_and_goes_stale_when_it_stops_differing() {
        let list = allowlist("| D-003 | base | get | * | * | * | the reference answers 500 here |");
        let both = [
            diff("base", "get", "status", "500", "404"),
            diff("base", "get", "body", "x", "y"),
        ];
        let verdict = judge(&both, &list, &ran());
        assert!(verdict.passed());
        assert_eq!(verdict.listed.len(), 2);
        assert!(!judge(&[diff("base", "other", "status", "500", "404")], &list, &ran()).passed());
        assert_eq!(judge(&[], &list, &ran()).stale, ["D-003"]);
    }

    #[test]
    fn a_file_without_the_markers_lists_nothing_and_a_short_row_is_an_error() {
        assert!(parse("just prose").expect("parse").is_empty());
        assert!(parse(&format!("{BEGIN}\n| D-001 | base |\n{END}")).is_err());
    }
}
