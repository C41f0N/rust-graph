use std::sync::RwLock;

pub struct AutocompleteState {
    pub active: bool,
    pub filter: String,
    pub selected: usize,
    pub matches: Vec<String>,
    pub esc_consumed: bool,
    pub suppress: bool,
    pub suppress_filter: String,
}

pub static AUTOCOMPLETE: RwLock<AutocompleteState> = RwLock::new(AutocompleteState {
    active: false,
    filter: String::new(),
    selected: 0,
    matches: Vec::new(),
    esc_consumed: false,
    suppress: false,
    suppress_filter: String::new(),
});

// If the cursor sits inside an unclosed [[ ... ]], return the text typed
// between "[[" and the cursor (the current completion filter).
pub fn detect(line: &str, cursor_x: usize) -> Option<String> {
    let bytes = line.as_bytes();
    let upto = cursor_x.min(bytes.len());
    let mut last_open: Option<usize> = None;
    let mut i = 0;

    while i + 1 < upto {
        if bytes[i] == b'[' && bytes[i + 1] == b'[' {
            last_open = Some(i);
            i += 2;
        } else if bytes[i] == b']' && bytes[i + 1] == b']' {
            last_open = None;
            i += 2;
        } else {
            i += 1;
        }
    }

    last_open.map(|pos| line[pos + 2..upto].to_string())
}

pub fn match_names(names: &[String], filter: &str) -> Vec<String> {
    let f = filter.to_lowercase();
    names
        .iter()
        .filter(|n| n.to_lowercase().starts_with(&f))
        .cloned()
        .collect()
}

pub fn refresh(buffer: &[String], cursor_y: usize, cursor_x: usize) {
    let filter = if cursor_y < buffer.len() {
        detect(&buffer[cursor_y], cursor_x)
    } else {
        None
    };

    let mut state = AUTOCOMPLETE.write().unwrap();
    state.esc_consumed = false;

    // Stay hidden after ESC until the user types again (filter changes).
    if state.suppress && filter.as_deref() == Some(state.suppress_filter.as_str()) {
        state.filter = filter.unwrap_or_default();
        state.active = false;
        state.matches.clear();
        state.selected = 0;
        return;
    }
    state.suppress = false;

    state.active = false;
    state.matches.clear();
    state.selected = 0;

    if let Some(f) = filter {
        state.filter = f;
        let names = {
            let nodes = crate::graph::processing::NODES.read().unwrap();
            let editing = crate::graph::processing::EDITING_NODE.read().unwrap();
            let current = (*editing)
                .and_then(|i| nodes.get(i))
                .map(|n| n.name.clone());
            nodes
                .iter()
                .filter(|n| Some(n.name.clone()) != current)
                .map(|n| n.name.clone())
                .collect::<Vec<String>>()
        };
        state.matches = match_names(&names, &state.filter);
        state.active = !state.matches.is_empty();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_inside_link() {
        let line = "see [[alpha";
        assert_eq!(detect(line, line.len()).unwrap(), "alpha");
    }

    #[test]
    fn detect_cursor_partway_into_link() {
        let line = "[[par";
        assert_eq!(detect(line, 4).unwrap(), "pa");
    }

    #[test]
    fn detect_closed_link() {
        let line = "[[done]] here";
        assert_eq!(detect(line, line.len()), None);
    }

    #[test]
    fn detect_last_open_wins() {
        let line = "[[a]] then [[b";
        assert_eq!(detect(line, line.len()).unwrap(), "b");
    }

    #[test]
    fn match_prefix_case_insensitive() {
        let names = vec![
            "Alpha.md".to_string(),
            "beta.md".to_string(),
            "alice.md".to_string(),
        ];
        assert_eq!(match_names(&names, "al"), vec!["Alpha.md", "alice.md"]);
    }
}