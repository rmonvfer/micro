//! Fuzzy matching for the menus that filter as you type.

/// Characters that make the position after them the start of a word.
const WORD_SEPARATORS: [char; 6] = ['-', '_', '.', '/', ':', ' '];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Match {
    pub score: f64,
}

/// Score `text` against `query`, or `None` when the query does not appear in order.
pub fn match_score(query: &str, text: &str) -> Option<Match> {
    let query: Vec<char> = query.to_lowercase().chars().collect();
    let text: Vec<char> = text.to_lowercase().chars().collect();

    if let Some(found) = score_in_order(&query, &text) {
        return Some(found);
    }

    
    let swapped = swap_letters_and_digits(&query)?;
    score_in_order(&swapped, &text).map(|found| Match {
        score: found.score + 5.0,
    })
}

fn score_in_order(query: &[char], text: &[char]) -> Option<Match> {
    if query.is_empty() {
        return Some(Match { score: 0.0 });
    }
    if query.len() > text.len() {
        return None;
    }

    let mut index = 0;
    let mut score = 0.0;
    let mut last_match: Option<usize> = None;
    let mut consecutive = 0;

    for (position, character) in text.iter().enumerate() {
        if index >= query.len() {
            break;
        }
        if *character != query[index] {
            continue;
        }

        let at_word_start = position == 0
            || text.get(position - 1).is_some_and(|previous| {
                WORD_SEPARATORS.contains(previous) || previous.is_whitespace()
            });

        match last_match {
            
            Some(previous) if previous + 1 == position => {
                consecutive += 1;
                score -= consecutive as f64 * 5.0;
            }
            Some(previous) => {
                consecutive = 0;
                score += (position - previous - 1) as f64 * 2.0;
            }
            None => consecutive = 0,
        }

        if at_word_start {
            score -= 10.0;
        }
        score += position as f64 * 0.1;

        last_match = Some(position);
        index += 1;
    }

    if index < query.len() {
        return None;
    }
    if query == text {
        score -= 100.0;
    }
    Some(Match { score })
}

/// `opus5` becomes `5opus` and back again; anything else has no swap to try.
fn swap_letters_and_digits(query: &[char]) -> Option<Vec<char>> {
    let letters = query.iter().take_while(|c| c.is_ascii_alphabetic()).count();
    let digits = query.iter().take_while(|c| c.is_ascii_digit()).count();

    if letters > 0 && letters < query.len() && query[letters..].iter().all(char::is_ascii_digit) {
        return Some(
            query[letters..]
                .iter()
                .chain(&query[..letters])
                .copied()
                .collect(),
        );
    }
    if digits > 0 && digits < query.len() && query[digits..].iter().all(char::is_ascii_alphabetic) {
        return Some(
            query[digits..]
                .iter()
                .chain(&query[..digits])
                .copied()
                .collect(),
        );
    }
    None
}

/// Keep the items whose text matches every token of `query`, best first.
pub fn filter<T, F>(items: Vec<T>, query: &str, text_of: F) -> Vec<T>
where
    F: Fn(&T) -> String,
{
    let tokens: Vec<&str> = query
        .trim()
        .split(|c: char| c.is_whitespace() || c == '/')
        .filter(|token| !token.is_empty())
        .collect();
    if tokens.is_empty() {
        return items;
    }

    let mut scored: Vec<(f64, T)> = Vec::new();
    for item in items {
        let text = text_of(&item);
        let mut total = 0.0;
        let mut matched = true;
        for token in &tokens {
            match match_score(token, &text) {
                Some(found) => total += found.score,
                None => {
                    matched = false;
                    break;
                }
            }
        }
        if matched {
            scored.push((total, item));
        }
    }

    
    scored.sort_by(|left, right| left.0.total_cmp(&right.0));
    scored.into_iter().map(|(_, item)| item).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names<'a>(items: Vec<&'a str>, query: &str) -> Vec<&'a str> {
        filter(items, query, |item| item.to_string())
    }

    #[test]
    fn an_empty_query_matches_anything() {
        assert!(match_score("", "whatever").is_some());
        assert_eq!(names(vec!["a", "b"], ""), vec!["a", "b"]);
        assert_eq!(names(vec!["a", "b"], "   "), vec!["a", "b"]);
    }

    #[test]
    fn characters_must_appear_in_order() {
        assert!(match_score("mdl", "model").is_some());
        assert!(match_score("mod", "model").is_some());
        assert!(match_score("ldm", "model").is_none());
        assert!(match_score("modelx", "model").is_none());
    }

    #[test]
    fn matching_ignores_case() {
        assert!(match_score("MOD", "model").is_some());
        assert!(match_score("mod", "MODEL").is_some());
    }

    #[test]
    fn an_exact_match_beats_a_partial_one() {
        let exact = match_score("model", "model").unwrap().score;
        let partial = match_score("model", "models").unwrap().score;
        assert!(exact < partial, "{exact} should beat {partial}");
    }

    #[test]
    fn a_prefix_beats_a_scattered_match() {
        let prefix = match_score("com", "compact").unwrap().score;
        let scattered = match_score("com", "clear-of-me").unwrap().score;
        assert!(prefix < scattered, "{prefix} should beat {scattered}");
    }

    #[test]
    fn a_word_boundary_beats_a_match_inside_a_word() {
        let boundary = match_score("s", "micro-session").unwrap().score;
        let inside = match_score("s", "sessions").unwrap().score;
        
        assert!(inside < boundary);

        let start = match_score("m", "model").unwrap().score;
        let middle = match_score("m", "compact").unwrap().score;
        assert!(start < middle, "a word start should beat a letter inside");
    }

    #[test]
    fn filtering_ranks_the_closest_candidate_first() {
        let commands = vec!["compact", "clear", "cwd", "model"];
        assert_eq!(
            names(commands.clone(), "c"),
            vec!["compact", "clear", "cwd"]
        );
        assert_eq!(names(commands.clone(), "cle"), vec!["clear"]);
        assert_eq!(names(commands, "zz"), Vec::<&str>::new());
    }

    #[test]
    fn every_token_has_to_match() {
        let models = vec!["anthropic/claude-opus-5", "google/gemini-2.5-pro"];
        assert_eq!(
            names(models.clone(), "anthropic opus"),
            vec!["anthropic/claude-opus-5"]
        );
        
        assert_eq!(
            names(models.clone(), "anthropic/opus"),
            vec!["anthropic/claude-opus-5"]
        );
        assert!(names(models, "anthropic gemini").is_empty());
    }

    #[test]
    fn a_digit_and_letter_swap_still_matches() {
        assert!(match_score("5opus", "claude-opus-5").is_some());
        assert!(match_score("opus5", "claude-5-opus").is_some());

        
        let typed = match_score("opus5", "opus5").unwrap().score;
        let swapped = match_score("5opus", "opus5").unwrap().score;
        assert!(typed < swapped);
    }

    #[test]
    fn candidates_that_score_alike_keep_their_declared_order() {
        let items = vec!["aa", "ab", "ac"];
        assert_eq!(names(items, "a"), vec!["aa", "ab", "ac"]);
    }
}
