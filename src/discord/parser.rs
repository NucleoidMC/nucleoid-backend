enum ParseState {
    Whitespace {
        expecting: bool,
    },
    String {
        s: String,
    },
    QuotedString {
        s: String,
        quote: char,
        prev_char: char,
    },
}

pub enum ParseString {
    Unquoted(String),
    Quoted {
        s: String,
        quote: char,
    }
}

pub fn parse_command(s: &str) -> Option<Vec<ParseString>> {
    // TODO: some *probably* unnecessary usages of s.clone() in here :/

    let mut result = vec![];

    let mut state = ParseState::Whitespace {
        expecting: false,
    };

    for c in s.chars() {
        let m_state = &mut state;
        match m_state {
            ParseState::Whitespace { expecting } => {
                if c.is_ascii_whitespace() {
                    continue;
                } else if *expecting {
                    return None; // fail parsing
                } else if c == '"' || c == '\'' {
                    state = ParseState::QuotedString { s: String::new(), quote: c, prev_char: '\0' };
                } else {
                    let mut s = String::new();
                    s.push(c);
                    state = ParseState::String { s };
                }
            },
            ParseState::String { s } => {
                if c.is_ascii_whitespace() {
                    result.push(ParseString::Unquoted(s.clone()));
                    state = ParseState::Whitespace { expecting: false };
                } else {
                    s.push(c);
                }
            },
            ParseState::QuotedString { s, quote, prev_char } => {
                if c == *quote && *prev_char != '\\' {
                    result.push(ParseString::Quoted { quote: *quote, s: s.clone() });
                    state = ParseState::Whitespace { expecting: true };
                } else {
                    s.push(c);
                    *prev_char = c;
                }
            },
        }
    }

    Some(result)
}
