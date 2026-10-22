//! What the switcher shows and which card is selected. No GUI code here, so
//! it is tested like any other logic.
//!
//! `mickey source` starts the switcher as
//!
//! ```text
//! mickey-ui switcher [--at <corner>] <focused> <pc>...
//! ```
//!
//! `<corner>` is `center`, `top-left`, `top-right`, `bottom-left` or
//! `bottom-right` (the default).
//!
//! where each `<pc>` is `online:<name>` or `offline:<name>`, the first one is
//! this PC, and `<focused>` is the index of the PC that has the focus. The
//! switcher prints the index of the picked PC and exits 0, or prints nothing
//! and exits 1 when closed without a choice.

use crate::place::Corner;

#[derive(Debug, PartialEq, Eq)]
pub struct Pc {
    pub name: String,
    pub online: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Choice {
    pub corner: Corner,
    pub pcs: Vec<Pc>,
    pub focused: usize,
    pub selected: usize,
}

impl Choice {
    pub fn from_args(args: &[String]) -> Result<Choice, String> {
        let (corner, args) = match args {
            [command, flag, corner, rest @ ..] if flag == "--at" => {
                let corner = Corner::parse(corner)
                    .ok_or_else(|| format!("`{corner}` is not a corner; {USAGE}"))?;
                let mut args = vec![command.clone()];
                args.extend_from_slice(rest);
                (corner, args)
            }
            _ => (Corner::BottomRight, args.to_vec()),
        };
        let [command, focused, pcs @ ..] = args.as_slice() else {
            return Err(USAGE.into());
        };
        if command != "switcher" || pcs.is_empty() {
            return Err(USAGE.into());
        }
        let pcs = pcs
            .iter()
            .map(|pc| parse_pc(pc))
            .collect::<Result<Vec<_>, _>>()?;
        let focused: usize = focused
            .parse()
            .ok()
            .filter(|&index| index < pcs.len())
            .ok_or_else(|| format!("focused PC `{focused}` is not one of the {} PCs", pcs.len()))?;
        let mut choice = Choice {
            corner,
            pcs,
            focused,
            selected: focused,
        };
        // Start on the next PC, so a quick Enter does what the hotkey did
        // before the switcher existed.
        choice.move_by(1);
        Ok(choice)
    }

    /// Moves the selection by `step` cards, wrapping around and skipping
    /// offline PCs. This PC (index 0) is always online.
    pub fn move_by(&mut self, step: isize) {
        let len = self.pcs.len() as isize;
        let mut index = self.selected as isize;
        for _ in 0..len {
            index = (index + step).rem_euclid(len);
            if self.pcs[index as usize].online {
                self.selected = index as usize;
                return;
            }
        }
    }

    /// The PC to report, if it can take the focus.
    pub fn pick(&self, index: usize) -> Option<usize> {
        self.pcs.get(index).filter(|pc| pc.online).map(|_| index)
    }
}

const USAGE: &str =
    "usage: mickey-ui switcher [--at <corner>] <focused> online:<name>|offline:<name>...";

fn parse_pc(arg: &str) -> Result<Pc, String> {
    let (state, name) = arg
        .split_once(':')
        .ok_or_else(|| format!("`{arg}` is not online:<name> or offline:<name>"))?;
    let online = match state {
        "online" => true,
        "offline" => false,
        _ => return Err(format!("`{state}` is not online or offline")),
    };
    Ok(Pc {
        name: name.to_string(),
        online,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choice(line: &str) -> Result<Choice, String> {
        let args: Vec<String> = line.split_whitespace().map(String::from).collect();
        Choice::from_args(&args)
    }

    #[test]
    fn starts_on_the_next_online_pc() {
        let c = choice("switcher 0 online:Laptop offline:Desktop online:Windows").unwrap();
        assert_eq!((c.focused, c.selected), (0, 2), "Desktop is offline");
        let c = choice("switcher 2 online:Laptop offline:Desktop online:Windows").unwrap();
        assert_eq!(c.selected, 0, "wraps around to this PC");
    }

    #[test]
    fn moving_skips_offline_pcs_both_ways() {
        let mut c = choice("switcher 0 online:A online:B offline:C online:D").unwrap();
        assert_eq!(c.selected, 1);
        c.move_by(1);
        assert_eq!(c.selected, 3);
        c.move_by(-1);
        assert_eq!(c.selected, 1);
        c.move_by(-1);
        assert_eq!(c.selected, 0);
    }

    #[test]
    fn offline_pcs_cannot_be_picked() {
        let c = choice("switcher 0 online:A offline:B").unwrap();
        assert_eq!(c.pick(0), Some(0));
        assert_eq!(c.pick(1), None);
        assert_eq!(c.pick(9), None);
    }

    #[test]
    fn the_corner_is_optional_and_checked() {
        assert_eq!(
            choice("switcher 0 online:A").unwrap().corner,
            Corner::BottomRight
        );
        let c = choice("switcher --at top-left 0 online:A online:B").unwrap();
        assert_eq!((c.corner, c.selected), (Corner::TopLeft, 1));
        assert!(choice("switcher --at middle 0 online:A").is_err());
    }

    #[test]
    fn bad_arguments_are_errors() {
        assert!(choice("switcher").is_err());
        assert!(choice("switcher 0").is_err());
        assert!(choice("switcher 5 online:A").is_err());
        assert!(choice("switcher 0 maybe:A").is_err());
        assert!(choice("settings 0 online:A").is_err());
    }

    #[test]
    fn names_may_contain_colons_and_spaces_come_in_one_argument() {
        let args = vec![
            "switcher".into(),
            "0".into(),
            "online:This PC".into(),
            "online:a:b".into(),
        ];
        let c = Choice::from_args(&args).unwrap();
        assert_eq!(c.pcs[0].name, "This PC");
        assert_eq!(c.pcs[1].name, "a:b");
    }
}
