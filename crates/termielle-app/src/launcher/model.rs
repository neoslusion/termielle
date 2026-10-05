use crate::animation::FrameBuffer;
use std::sync::Arc;

pub(super) const MAX_RESULTS: usize = 6;

pub(super) struct App {
    pub name: String,
    pub key: String,
    pub target: Vec<u8>,
    pub icon: Option<Arc<FrameBuffer>>,
}

pub(super) struct Model {
    pub apps: Vec<App>,
    pub query: String,
    pub results: Vec<usize>,
    pub selected: usize,
    pub indexing: bool,
    pub launching: bool,
    pub error: Option<String>,
}

impl Default for Model {
    fn default() -> Self {
        Self {
            apps: Vec::new(),
            query: String::new(),
            results: Vec::new(),
            selected: 0,
            indexing: true,
            launching: false,
            error: None,
        }
    }
}

impl Model {
    pub fn set_apps(&mut self, apps: Vec<App>) {
        self.apps = apps;
        self.indexing = false;
        self.search();
    }

    pub fn set_query(&mut self, query: String) {
        self.query = query;
        self.error = None;
        self.search();
    }

    fn search(&mut self) {
        let query = self.query.to_lowercase();
        let tokens: Vec<_> = query.split_whitespace().collect();
        let mut matches: Vec<_> = self
            .apps
            .iter()
            .enumerate()
            .filter_map(|(index, app)| {
                let score = tokens.iter().try_fold(0usize, |total, token| {
                    match_score(&app.key, token).map(|score| total + score)
                })?;
                Some((score, &app.key, index))
            })
            .collect();
        matches.sort_unstable();
        self.results = matches
            .into_iter()
            .take(MAX_RESULTS)
            .map(|(_, _, index)| index)
            .collect();
        self.selected = 0;
        for (index, app) in self.apps.iter_mut().enumerate() {
            if !self.results.contains(&index) {
                app.icon = None;
            }
        }
    }

    pub fn move_selection(&mut self, delta: i32) {
        if !self.results.is_empty() {
            self.selected =
                (self.selected as i32 + delta).rem_euclid(self.results.len() as i32) as usize;
        }
    }

    pub fn selected_app(&self) -> Option<&App> {
        self.results
            .get(self.selected)
            .and_then(|index| self.apps.get(*index))
    }
}

fn match_score(name: &str, query: &str) -> Option<usize> {
    if name == query {
        return Some(0);
    }
    if name.starts_with(query) {
        return Some(10 + name.chars().count().saturating_sub(query.chars().count()));
    }
    if let Some(position) = name.find(query) {
        let boundary = name[..position]
            .chars()
            .next_back()
            .is_some_and(|letter| !letter.is_alphanumeric());
        return Some(if boundary {
            100 + position
        } else {
            200 + position
        });
    }
    let initials: String = name
        .split(|letter: char| !letter.is_alphanumeric())
        .filter_map(|word| word.chars().next())
        .collect();
    if initials.starts_with(query) {
        return Some(150 + initials.chars().count());
    }
    let mut letters = query.chars();
    let mut next = letters.next()?;
    let mut first = None;
    for (position, letter) in name.chars().enumerate() {
        if letter == next {
            let start = *first.get_or_insert(position);
            if let Some(letter) = letters.next() {
                next = letter;
            } else {
                let span = position - start;
                return (span <= (query.chars().count() * 2).max(6))
                    .then_some(1_000 + span + start * 2);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_changes_release_artwork_for_hidden_results() {
        let mut model = Model::default();
        model.set_apps(apps(&["Alpha", "Beta", "Gamma"]));
        let icon = Arc::new(FrameBuffer {
            width: 1,
            height: 1,
            pixels_pbgra: vec![255; 4],
            delay_ms: 0,
            loop_index: 0,
            scale: 1.0,
        });
        for app in &mut model.apps {
            app.icon = Some(icon.clone());
        }
        model.set_query("beta".into());
        assert!(model.apps[0].icon.is_none());
        assert!(Arc::ptr_eq(model.apps[1].icon.as_ref().unwrap(), &icon));
        assert!(model.apps[2].icon.is_none());
        assert_eq!(Arc::strong_count(&icon), 2);
    }

    fn apps(names: &[&str]) -> Vec<App> {
        names
            .iter()
            .map(|name| App {
                name: (*name).into(),
                key: name.to_lowercase(),
                target: vec![0, 0],
                icon: None,
            })
            .collect()
    }

    #[test]
    fn exact_prefix_word_and_fuzzy_matches_have_stable_priority() {
        let mut model = Model::default();
        model.set_apps(apps(&[
            "Visual Studio Code",
            "Discord",
            "Code",
            "Code Editor",
            "Barcode Tool",
        ]));
        model.set_query("CODE".into());
        let names: Vec<_> = model
            .results
            .iter()
            .map(|index| model.apps[*index].name.as_str())
            .collect();
        assert_eq!(
            names,
            ["Code", "Code Editor", "Visual Studio Code", "Barcode Tool"]
        );
        model.set_query("vsc".into());
        assert_eq!(model.selected_app().unwrap().name, "Visual Studio Code");
    }

    #[test]
    fn loose_subsequences_do_not_turn_unrelated_apps_into_matches() {
        let mut model = Model::default();
        model.set_apps(apps(&[
            "Microsoft Edge",
            "Recovery Drive",
            "Visual Studio Code",
            "Google Chrome",
        ]));
        model.set_query("code".into());
        assert_eq!(model.results.len(), 1);
        assert_eq!(model.selected_app().unwrap().name, "Visual Studio Code");
        model.set_query("chrme".into());
        assert_eq!(model.selected_app().unwrap().name, "Google Chrome");
    }

    #[test]
    fn multiple_tokens_unicode_and_whitespace_work_without_io() {
        let mut model = Model::default();
        model.set_apps(apps(&[
            "Éditeur 日本語",
            "Visual Studio Code",
            "Studio Music",
        ]));
        model.set_query("  code   visual  ".into());
        assert_eq!(model.selected_app().unwrap().name, "Visual Studio Code");
        model.set_query("ÉDI 日".into());
        assert_eq!(model.selected_app().unwrap().name, "Éditeur 日本語");
        model.set_query("missing app".into());
        assert!(model.selected_app().is_none());
        model.move_selection(1);
        assert_eq!(model.selected, 0);
    }

    #[test]
    fn selection_wraps_and_query_or_catalog_changes_reset_it() {
        let mut model = Model::default();
        model.set_apps(apps(&["Zulu", "Alpha", "Beta"]));
        assert_eq!(model.selected_app().unwrap().name, "Alpha");
        model.move_selection(-1);
        assert_eq!(model.selected_app().unwrap().name, "Zulu");
        model.move_selection(1);
        assert_eq!(model.selected_app().unwrap().name, "Alpha");
        model.move_selection(1);
        model.set_query("z".into());
        assert_eq!(model.selected, 0);
        model.set_apps(apps(&["Zen"]));
        assert_eq!(model.query, "z");
        assert_eq!(model.selected_app().unwrap().name, "Zen");
    }

    #[test]
    fn results_are_bounded_and_large_catalog_search_is_interactive() {
        let mut model = Model::default();
        model.set_apps(
            (0..5_000)
                .map(|index| App {
                    name: format!("Application {index}"),
                    key: format!("application {index}"),
                    target: vec![0, 0],
                    icon: None,
                })
                .collect(),
        );
        let started = std::time::Instant::now();
        model.set_query("application 42".into());
        assert_eq!(model.results.len(), MAX_RESULTS);
        assert_eq!(model.selected_app().unwrap().name, "Application 42");
        println!("Filtered 5,000 app names in {:?}", started.elapsed());
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }
}
