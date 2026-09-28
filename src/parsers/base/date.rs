use crate::formats::date::MultiDateFormatParser;
use crate::structs::TextItem;

/// DateParser: parses date strings using multiple date formats.
pub struct DateParser {
    /// The current parsed UTC timestamp (milliseconds since epoch)
    pub value: Option<i64>,
    /// Dispatcher for multiple date formats
    pub parser: MultiDateFormatParser,
    /// Supported candidate whitespace-term counts, largest first
    item_counts: Vec<usize>,
    /// Maximum number of space-delimited items in the selected formats
    pub max_lookahead: usize,
    /// A copy of the last successfully parsed text item (merged text)
    pub text_item: Option<TextItem>,
}

impl DateParser {
    /// Create a new DateParser with specified format names
    pub fn new(format_names: &[&str]) -> Self {
        let parser = MultiDateFormatParser::new(format_names);
        let max_lookahead = parser.max_items();
        let item_counts = parser.item_counts().to_vec();
        DateParser {
            value: None,
            parser,
            item_counts,
            max_lookahead,
            text_item: None,
        }
    }

    /// Reset the parser state
    pub fn reset(&mut self) {
        self.value = None;
        self.text_item = None;
    }

    /// Iteratively join text items and attempt to parse dates
    /// Returns number of items consumed if successful, else 0
    pub fn parse_items(&mut self, items: &[TextItem], year_str: &str) -> usize {
        self.parse_items_filtered(items, year_str, |_| true)
    }

    /// Same as `parse_items`, but rejects the candidate up front if it fails
    /// `predicate`, before running the (comparatively expensive) date parsing.
    /// Note: the resulting text item's position always derives from `items[0]`,
    /// so the predicate result is invariant across all candidate lengths.
    pub fn parse_items_filtered<F>(
        &mut self,
        items: &[TextItem],
        year_str: &str,
        predicate: F,
    ) -> usize
    where
        F: Fn(&TextItem) -> bool,
    {
        if items.is_empty() || !predicate(&items[0]) {
            return 0;
        }
        // Try longest first, then shorter
        for term_count in &self.item_counts {
            let Some(i) = Self::items_for_term_count(items, *term_count) else {
                continue;
            };
            let merged = items[0..i]
                .iter()
                .map(|t| t.text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            if let Some(val) = self.parser.parse(&merged, year_str, *term_count) {
                self.value = Some(val);
                self.text_item = Some(TextItem {
                    text: merged,
                    ..items[0].clone()
                });
                return i;
            }
        }
        0
    }

    fn items_for_term_count(items: &[TextItem], term_count: usize) -> Option<usize> {
        let mut accumulated_terms = 0;
        for (index, item) in items.iter().enumerate() {
            accumulated_terms += item.text.split_whitespace().count();
            if accumulated_terms == term_count {
                return Some(index + 1);
            }
            if accumulated_terms > term_count {
                return None;
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn make_text_item(text: &str) -> TextItem {
        TextItem {
            text: text.to_string(),
            x1: 0,
            y1: 0,
            x2: 0,
            y2: 0,
            page: 1,
            y1_bin: 0,
        }
    }

    #[test]
    fn test_parse_single_item_format1() {
        let mut parser = DateParser::new(&["format1"]);
        let items = vec![make_text_item("24 mar")];
        let consumed = parser.parse_items(&items, "2023");
        assert_eq!(consumed, 1);
        assert!(parser.value.is_some());
        assert_eq!(parser.text_item.as_ref().unwrap().text, "24 mar");
    }

    #[test]
    fn test_item_counts_are_distinct_and_descending() {
        let parser = DateParser::new(&["format1", "format2", "format10"]);
        assert_eq!(parser.item_counts, vec![3, 2]);
    }

    #[test]
    fn test_parse_multiple_items_format2() {
        let mut parser = DateParser::new(&["format2"]);
        let items = vec![
            make_text_item("24"),
            make_text_item("march"),
            make_text_item("2020"),
        ];
        let consumed = parser.parse_items(&items, "");
        assert_eq!(consumed, 3);
        assert!(parser.value.is_some());
        assert_eq!(parser.text_item.as_ref().unwrap().text, "24 march 2020");
    }

    #[test]
    fn test_no_match() {
        let mut parser = DateParser::new(&["format1"]);
        let items = vec![make_text_item("foo")];
        let consumed = parser.parse_items(&items, "2023");
        assert_eq!(consumed, 0);
        assert!(parser.value.is_none());
        assert!(parser.text_item.is_none());
    }

    #[test]
    fn test_reset() {
        let mut parser = DateParser::new(&["format1"]);
        let items = vec![make_text_item("24 mar")];
        parser.parse_items(&items, "2023");
        assert!(parser.value.is_some());
        parser.reset();
        assert!(parser.value.is_none());
        assert!(parser.text_item.is_none());
    }
}
