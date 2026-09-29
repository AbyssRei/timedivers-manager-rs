use std::path::Path;

use indexmap::IndexMap;

/// A parsed `appmanifest_*.acf` value: either a scalar string or a nested map.
#[derive(Debug, Clone, PartialEq)]
pub enum AcfValue {
    Str(String),
    Map(AcfMap),
}

impl AcfValue {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            AcfValue::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_map(&self) -> Option<&AcfMap> {
        match self {
            AcfValue::Map(m) => Some(m),
            _ => None,
        }
    }
}

/// An ordered map of ACF keys to values (ACF files list depots in a
/// meaningful order, so insertion order is preserved).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AcfMap {
    entries: IndexMap<String, AcfValue>,
}

impl AcfMap {
    pub fn get(&self, key: &str) -> Option<&AcfValue> {
        self.entries.get(key)
    }

    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(AcfValue::as_str)
    }

    pub fn get_map(&self, key: &str) -> Option<&AcfMap> {
        self.get(key).and_then(AcfValue::as_map)
    }

    fn from_entries(entries: IndexMap<String, AcfValue>) -> Self {
        Self { entries }
    }

    /// Ordered keys of a nested map (used for `InstalledDepots`).
    pub fn key_list(&self, key: &str) -> Vec<String> {
        self.get_map(key)
            .map(|m| m.entries.keys().cloned().collect())
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Str(String),
    LBrace,
    RBrace,
}

fn tokenize(text: &str) -> Vec<Token> {
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                let start = i + 1;
                let mut end = start;
                while end < bytes.len() && bytes[end] != b'"' {
                    end += 1;
                }
                tokens.push(Token::Str(text[start..end].to_string()));
                i = end + 1;
            }
            b'{' => {
                tokens.push(Token::LBrace);
                i += 1;
            }
            b'}' => {
                tokens.push(Token::RBrace);
                i += 1;
            }
            _ => i += 1,
        }
    }
    tokens
}

fn parse_obj(tokens: &[Token], pos: &mut usize) -> AcfMap {
    let mut entries = IndexMap::new();
    while *pos < tokens.len() {
        match &tokens[*pos] {
            Token::RBrace => {
                *pos += 1;
                return AcfMap::from_entries(entries);
            }
            Token::Str(key) => {
                let key = key.clone();
                *pos += 1;
                if *pos < tokens.len() {
                    match &tokens[*pos] {
                        Token::LBrace => {
                            *pos += 1;
                            let value = parse_obj(tokens, pos);
                            entries.insert(key, AcfValue::Map(value));
                        }
                        Token::Str(value) => {
                            entries.insert(key, AcfValue::Str(value.clone()));
                            *pos += 1;
                        }
                        Token::RBrace => {
                            *pos += 1;
                            return AcfMap::from_entries(entries);
                        }
                    }
                }
            }
            Token::LBrace => *pos += 1,
        }
    }
    AcfMap::from_entries(entries)
}

/// Parses the content of an `appmanifest_*.acf` file (VDF subset), equivalent
/// to the original implementation's tokenizer + recursive `parse_obj`.
pub fn parse_acf_text(text: &str) -> AcfMap {
    let tokens = tokenize(text);
    let mut pos = 0;
    if !tokens.is_empty() && matches!(&tokens[0], Token::Str(_)) {
        pos = 1;
    }
    if pos < tokens.len() && tokens[pos] == Token::LBrace {
        pos += 1;
        parse_obj(&tokens, &mut pos)
    } else {
        AcfMap::default()
    }
}

pub fn parse_acf(path: &Path) -> anyhow::Result<AcfMap> {
    // Steam writes ACF files in UTF-8; stray bytes are tolerated like the
    // Python version (errors="ignore").
    let bytes = std::fs::read(path)?;
    let text = String::from_utf8_lossy(&bytes);
    Ok(parse_acf_text(&text))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
"AppState"
{
	"appid"		"553850"
	"Universe"		"1"
	"name"		"HELLDIVERS 2"
	"StateFlags"		"4"
	"installdir"		"Helldivers 2"
	"InstalledDepots"
	{
		"553851"
		{
			"ManifestID"		"1234567890"
		}
		"553852"
		{
			"ManifestID"		"9876543210"
		}
	}
	"UserConfig"
	{
		"BetaKey"		"public"
	}
	"MountedDepots"
	{
	}
}
"#;

    #[test]
    fn parses_top_level_fields() {
        let acf = parse_acf_text(SAMPLE);
        assert_eq!(acf.get_str("appid"), Some("553850"));
        assert_eq!(acf.get_str("name"), Some("HELLDIVERS 2"));
        assert_eq!(acf.get_str("installdir"), Some("Helldivers 2"));
    }

    #[test]
    fn preserves_depot_order() {
        let acf = parse_acf_text(SAMPLE);
        assert_eq!(acf.key_list("InstalledDepots"), vec!["553851", "553852"]);
    }

    #[test]
    fn reads_nested_user_config() {
        let acf = parse_acf_text(SAMPLE);
        let user = acf.get_map("UserConfig").unwrap();
        assert_eq!(user.get_str("BetaKey"), Some("public"));
    }

    #[test]
    fn empty_maps_parse() {
        let acf = parse_acf_text(SAMPLE);
        assert!(acf.get_map("MountedDepots").unwrap().is_empty());
    }

    #[test]
    fn malformed_input_yields_empty() {
        assert!(parse_acf_text("garbage without braces").is_empty());
        assert!(parse_acf_text("").is_empty());
        let partial = parse_acf_text("\"AppState\" { \"appid\" \"1\"");
        assert_eq!(partial.get_str("appid"), Some("1"));
    }
}
