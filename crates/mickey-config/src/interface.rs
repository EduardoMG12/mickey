//! `[interface]`: where the switcher shows and how it orders the PCs.

use serde::Deserialize;

/// `[interface]`: how Mickey shows itself.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Interface {
    /// Where the switcher appears: a corner or `center`.
    #[serde(default = "default_switcher_position")]
    pub switcher_position: String,
    /// The switcher's cards in a column instead of a row.
    #[serde(default)]
    pub switcher_vertical: bool,
    /// PC names in the order the switcher shows them; PCs left out follow,
    /// as the layout places them. Empty: all as the layout places them.
    #[serde(default)]
    pub switcher_order: Vec<String>,
}

impl Default for Interface {
    fn default() -> Interface {
        Interface {
            switcher_position: default_switcher_position(),
            switcher_vertical: false,
            switcher_order: Vec::new(),
        }
    }
}

impl Interface {
    /// The switcher's cards in the order to show them: each card's name and
    /// its place in the layout (`None` if not placed). The custom order goes
    /// first; then left to right as the layout places them (top to bottom
    /// in a column); unplaced PCs last. Returns indexes into `cards`.
    pub fn switcher_sort(&self, cards: &[(&str, Option<[i32; 2]>)]) -> Vec<usize> {
        let mut order: Vec<usize> = (0..cards.len()).collect();
        order.sort_by_key(|&index| {
            let (name, place) = cards[index];
            let custom = self.switcher_order.iter().position(|wanted| wanted == name);
            let place = place.map(|[x, y]| {
                if self.switcher_vertical {
                    (y, x)
                } else {
                    (x, y)
                }
            });
            (custom.unwrap_or(usize::MAX), place.is_none(), place, index)
        });
        order
    }
}

pub const SWITCHER_POSITIONS: [&str; 5] = [
    "center",
    "top-left",
    "top-right",
    "bottom-left",
    "bottom-right",
];

fn default_switcher_position() -> String {
    "bottom-right".into()
}
