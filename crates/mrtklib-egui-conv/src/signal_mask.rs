//! The 108 RTKCONV signal choices, using MRTKLIB observation-code strings.
//! Tokens follow convbin.c setmask: system letter + 'L' + obs2code string.

use serde::{Deserialize, Serialize};

pub(crate) struct Group {
    pub name: &'static str,
    pub system: &'static str,
    pub codes: &'static [&'static str],
}

// Audited against MRTKLIB src/data/mrtk_obs.c obscodes and convbin.c:320-345.
// RTKCONV's Galileo caption 8I selects code 37; its actual string is 8L.
pub(crate) const GROUPS: [Group; 7] = [
    Group {
        name: "GPS",
        system: "G",
        codes: &[
            "1C", "1P", "1W", "1Y", "1M", "1N", "1S", "1L", "2C", "2D", "2S", "2L", "2X", "2P",
            "2W", "2Y", "2M", "2N", "5I", "5Q", "5X",
        ],
    },
    Group {
        name: "GLONASS",
        system: "R",
        codes: &[
            "1C", "1P", "2C", "2P", "3I", "3Q", "3X", "4A", "4B", "4X", "6A", "6B", "6X",
        ],
    },
    Group {
        name: "Galileo",
        system: "E",
        codes: &[
            "1C", "1A", "1B", "1X", "1Z", "5I", "5Q", "5X", "7I", "7Q", "7X", "6A", "6B", "6C",
            "6X", "6Z", "8L", "8Q", "8X",
        ],
    },
    Group {
        name: "QZSS",
        system: "J",
        codes: &[
            "1C", "1S", "1L", "1Z", "1X", "2S", "2L", "2X", "5I", "5Q", "5X", "5D", "5P", "5Z",
            "6S", "6L", "6X", "6E", "6Z",
        ],
    },
    Group {
        name: "SBAS",
        system: "S",
        codes: &["1C", "5I", "5Q", "5X"],
    },
    Group {
        name: "BDS",
        system: "C",
        codes: &[
            "2I", "2Q", "2X", "6I", "6Q", "6X", "7I", "7Q", "7X", "1D", "1P", "1X", "1A", "1N",
            "5D", "5P", "5X", "7D", "7P", "7Z", "8D", "8P", "8X", "6A",
        ],
    },
    Group {
        name: "NavIC",
        system: "I",
        codes: &["5A", "5B", "5C", "5X", "9A", "9B", "9C", "9X"],
    },
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct SignalMask(pub [Vec<bool>; 7]);

impl Default for SignalMask {
    fn default() -> Self {
        Self(std::array::from_fn(|i| vec![true; GROUPS[i].codes.len()]))
    }
}
impl SignalMask {
    pub fn validate_shape(&self) -> Result<(), String> {
        if self
            .0
            .iter()
            .zip(&GROUPS)
            .any(|(values, group)| values.len() != group.codes.len())
        {
            return Err("invalid Signal Mask checkbox count".into());
        }
        Ok(())
    }
    pub fn set_all(&mut self, checked: bool) {
        for values in &mut self.0 {
            values.fill(checked);
        }
    }
    pub fn allow_list(&self) -> Result<Option<String>, String> {
        self.validate_shape()?;
        if self.0.iter().flatten().all(|checked| *checked) {
            return Ok(None);
        }
        let mut tokens = Vec::new();
        for (values, group) in self.0.iter().zip(&GROUPS) {
            for (checked, code) in values.iter().zip(group.codes) {
                if *checked {
                    tokens.push(format!("{}L{code}", group.system));
                }
            }
        }
        if tokens.is_empty() {
            return Err("Select at least one signal code in Signal Mask".into());
        }
        Ok(Some(tokens.join(",")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_default_single_allow_list_and_empty_rejection() {
        let mut mask = SignalMask::default();
        assert_eq!(mask.0.iter().map(Vec::len).sum::<usize>(), 108);
        assert_eq!(mask.allow_list().unwrap(), None);
        mask.set_all(false);
        assert!(mask.allow_list().unwrap_err().contains("at least one"));
        mask.0[0][0] = true;
        mask.0[2][16] = true;
        mask.0[6][7] = true;
        assert_eq!(mask.allow_list().unwrap().unwrap(), "GL1C,EL8L,IL9X");
        mask.set_all(true);
        assert_eq!(mask.allow_list().unwrap(), None);
        mask.0[0].pop();
        assert!(mask.validate_shape().is_err());
    }
    #[test]
    fn every_choice_has_one_distinct_four_character_parser_token() {
        let mut mask = SignalMask::default();
        mask.0[0][0] = false;
        let mut tokens: Vec<_> = mask
            .allow_list()
            .unwrap()
            .unwrap()
            .split(',')
            .map(str::to_owned)
            .collect();
        tokens.push("GL1C".into());
        assert!(tokens.iter().all(|t| t.len() == 4
            && t.as_bytes()[1] == b'L'
            && "GREJSCI".contains(t.chars().next().unwrap())));
        tokens.sort();
        tokens.dedup();
        assert_eq!(tokens.len(), 108);
        assert!(tokens.contains(&"EL8L".into()));
        assert!(!tokens.contains(&"EL8I".into()));
    }
}
