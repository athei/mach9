//! Round trips of [`Table`] through the value of `WINE_COMPATDB`, on random
//! tables from a seeded generator and on every short text from a small
//! alphabet of the characters the format escapes or must keep as they are.
//!
//! The generator builds its tables through the public API alone, so every
//! table it makes is one a launcher could make. It is seeded, so a failure
//! names its seed and case and comes back the same on every run.

use compatdb_core::{
    Argument, DebugChannel, DllOverride, EnvEntry, EnvName, Exe, FieldError, Fingerprint, Key,
    KeyKind, Rule, Scalar, Table,
};

/// What a test returns, so a missing value fails it through `?` rather
/// than a panic in the test body.
type TestResult = Result<(), Box<dyn std::error::Error>>;

/// The pieces the generator joins into user data: the characters the
/// format escapes, those it must pass through as they are, words of the
/// format in another case, and plain text.
const POOL: &[&str] = &[
    "%", ";", "=", "\r", "\n", "\t", "\x7f", "é", "😀", " ", "*", "default", "Default", "DXMT",
    "True", "Game", "exe", "a", "%25", "%3B", "%zz", ",", "x.exe", "v=6",
];

/// The load orders of a generated DLL override; the user text of the pool
/// comes on top.
const ORDERS: &[&str] = &["n,b", "b", "n", "", "default", "DEFAULT"];

/// The Steam app ids of a generated `exe`.
const STEAM_APPS: &[u32] = &[1, 220, 440, u32::MAX];

/// A xorshift64* generator: small, fast and the same on every platform.
struct Gen(u64);

impl Gen {
    /// The generator for `seed`, which must not be zero.
    const fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// The next 64 random bits.
    const fn bits(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    /// A number below `n`, or 0 when `n` is 0.
    fn below(&mut self, n: usize) -> usize {
        let n = u64::try_from(n).unwrap_or(u64::MAX);
        if n == 0 {
            return 0;
        }
        usize::try_from(self.bits() % n).unwrap_or(0)
    }

    /// True once in `n` times.
    fn one_in(&mut self, n: usize) -> bool {
        self.below(n) == 0
    }

    /// One of `items`, or `None` when there is none.
    fn pick<'a, T>(&mut self, items: &'a [T]) -> Option<&'a T> {
        items.get(self.below(items.len()))
    }

    /// `text` with each ASCII letter in a random case.
    fn mangle_case(&mut self, text: &str) -> String {
        text.chars()
            .map(|c| {
                if self.one_in(2) {
                    c.to_ascii_uppercase()
                } else {
                    c.to_ascii_lowercase()
                }
            })
            .collect()
    }

    /// The value `text` of `WINE_COMPATDB` with each key, each value of a
    /// scalar setting and the `steam:` of an `exe` in a random case. User
    /// data keeps its case.
    fn mangle_words(&mut self, text: &str) -> String {
        let mut lines = Vec::new();
        for line in text.lines() {
            let mut fields = Vec::new();
            for field in line.split(';') {
                let Some((key, value)) = field.split_once('=') else {
                    fields.push(field.to_owned());
                    continue;
                };
                let is_word = key.parse::<Key>().is_ok_and(|key| match key.kind() {
                    KeyKind::Scalar(_) => true,
                    _ => key == Key::Exe && value.starts_with("steam:"),
                });
                let value = if is_word {
                    self.mangle_case(value)
                } else {
                    value.to_owned()
                };
                fields.push(format!("{}={value}", self.mangle_case(key)));
            }
            lines.push(fields.join(";"));
        }
        lines.join("\n")
    }

    /// User data: zero to four pieces of [`POOL`].
    fn text(&mut self) -> String {
        let pieces = self.below(5);
        (0..pieces)
            .filter_map(|_| self.pick(POOL).copied())
            .collect()
    }

    /// A part made by `make` from user data, drawn again until `make`
    /// takes it.
    fn part<T>(&mut self, make: impl Fn(String) -> Result<T, FieldError>) -> Result<T, String> {
        let mut last = None;
        for _ in 0..1000 {
            match make(self.text()) {
                Ok(part) => return Ok(part),
                Err(error) => last = Some(error),
            }
        }
        Err(format!(
            "no valid text in 1000 draws, the last one: {last:?}"
        ))
    }

    /// The entries of a list: zero to three parts made by `make`.
    fn list<T>(
        &mut self,
        make: impl Fn(String) -> Result<T, FieldError> + Copy,
    ) -> Result<Vec<T>, String> {
        let len = self.below(4);
        (0..len).map(|_| self.part(make)).collect()
    }

    /// A rule with every field chosen at random.
    ///
    /// The scalar settings go through [`Rule::set`] with a spelling of
    /// [`Key::values`] in a random case, `default` included, or stay unset;
    /// everything else goes through the typed parts and the `with_`
    /// methods.
    fn rule(&mut self) -> Result<Rule, Box<dyn std::error::Error>> {
        let exe = match self.below(8) {
            0 | 1 => Exe::Any,
            2 => {
                let app = self.pick(STEAM_APPS).ok_or("no app")?;
                Exe::new(format!("{}:{app}", self.mangle_case("steam")))?
            }
            _ => self.part(Exe::new)?,
        };
        let mut rule = Rule::new(exe);
        if self.one_in(3) {
            rule = rule.with_company(self.part(Fingerprint::new)?);
        }
        if self.one_in(3) {
            rule = rule.with_product(self.part(Fingerprint::new)?);
        }
        if self.one_in(3) {
            rule = rule.with_original_filename(self.part(Fingerprint::new)?);
        }
        for &scalar in Scalar::ALL {
            if self.one_in(3) {
                continue;
            }
            let value = self
                .pick(scalar.key().values())
                .ok_or("a scalar without values")?;
            let value = self.mangle_case(value);
            rule.set(scalar.key(), &value)?;
        }
        for over in self.list(|names| DllOverride::new(format!("{names}=")))? {
            let order = self.pick(ORDERS).copied().unwrap_or_default();
            let text = if self.one_in(4) {
                format!("{over}{}", self.text())
            } else {
                format!("{over}{order}")
            };
            rule = rule.with_dll_override(DllOverride::new(text)?);
        }
        for argument in self.list(Argument::new)? {
            rule = rule.with_argument(argument);
        }
        if self.one_in(2) {
            rule = rule.with_arguments_override(self.list(Argument::new)?);
        }
        for channel in self.list(DebugChannel::new)? {
            rule = rule.with_debug_channel(channel);
        }
        for name in self.list(EnvName::new)? {
            let entry = if self.one_in(3) {
                EnvEntry::remove(name)
            } else {
                EnvEntry::new(name, self.text())?
            };
            rule = rule.with_env(entry);
        }
        Ok(rule)
    }

    /// A table of zero to four random rules, pushed as a launcher pushes
    /// them.
    fn table(&mut self) -> Result<Table, Box<dyn std::error::Error>> {
        let mut table = Table::new();
        for _ in 0..self.below(5) {
            table.push(self.rule()?);
        }
        Ok(table)
    }
}

/// The checks every table that the public API can build passes, with
/// `case` naming it in a failure.
fn assert_round_trips(table: &Table, case: &str) {
    let text = table.to_string();
    assert_eq!(
        Table::parse_strict(&text),
        Ok(table.clone()),
        "{case}: {text:?}"
    );
    assert_eq!(
        Table::parse_lenient(&text),
        (table.clone(), Vec::new()),
        "{case}: {text:?}"
    );
    // Canonical text writes back byte for byte.
    let again = Table::parse_strict(&text).map(|parsed| parsed.to_string());
    assert_eq!(again.as_deref(), Ok(text.as_str()), "{case}");
    // One line per rule: the escapes keep CR and LF out of a record.
    assert_eq!(
        text.lines().count(),
        table.rules().len(),
        "{case}: {text:?}"
    );
    // The built-in rules never travel.
    assert_eq!(table.clone().with_builtin().to_string(), text, "{case}");
}

#[test]
fn random_tables_round_trip() -> TestResult {
    for seed in [1, 2, 3, 0x5eed, 0xdead_beef, 0x0123_4567_89ab_cdef] {
        let mut generator = Gen::new(seed);
        for case in 0..600 {
            let table = generator.table()?;
            let case = format!("seed {seed:#x}, case {case}");
            assert_round_trips(&table, &case);
            // The same text with its keys and words in random case reads
            // as the same table, and writes the canonical text again.
            let text = table.to_string();
            let loose = generator.mangle_words(&text);
            let parsed = Table::parse_strict(&loose);
            assert_eq!(parsed.as_ref(), Ok(&table), "{case}: {loose:?}");
            assert_eq!(
                parsed.map(|parsed| parsed.to_string()).as_deref(),
                Ok(text.as_str()),
                "{case}: {loose:?}"
            );
        }
    }
    Ok(())
}

#[test]
fn the_generator_reaches_every_key_and_setting_state() -> TestResult {
    // What the generated tables write, so that the suite above cannot
    // quietly stop covering a key, a reset, a removal or a list length.
    let mut generator = Gen::new(1);
    let mut text = String::new();
    // For each list, which lengths from 0 to 3 a rule had.
    let mut lengths = [[false; 4]; 5];
    for _ in 0..600 {
        let table = generator.table()?;
        for rule in table.rules() {
            let lists = [
                rule.dll_overrides().len(),
                rule.arguments().len(),
                rule.arguments_override().map_or(0, <[Argument]>::len),
                rule.debug_channels().len(),
                rule.env().len(),
            ];
            for (seen, length) in lengths.iter_mut().zip(lists) {
                if let Some(seen) = seen.get_mut(length) {
                    *seen = true;
                }
            }
        }
        text.push_str(&table.to_string());
        text.push('\n');
    }
    for key in Key::ALL {
        assert!(text.contains(&format!("{key}=")), "{key}");
        if matches!(key.kind(), KeyKind::Scalar(_)) {
            for value in key.values() {
                assert!(text.contains(&format!("{key}={value}")), "{key}={value}");
            }
        }
    }
    assert!(text.contains("exe=*"));
    assert!(text.contains("exe=steam:"));
    assert!(text.contains("arguments_override=\n") || text.contains("arguments_override=;"));
    assert!(text.lines().any(|line| {
        line.split(';')
            .any(|field| field.starts_with("env=") && field.ends_with('='))
    }));
    assert_eq!(lengths, [[true; 4]; 5]);
    Ok(())
}

/// The characters of the exhaustive passes: those the format escapes or
/// splits on, non-ASCII, and plain letters and digits.
const ALPHABET: &[char] = &[
    '%', ';', '=', '\r', '\n', '\t', '\x7f', 'é', '😀', ' ', ',', '*', 'a', 'D', '2', '5',
];

/// Every text of up to `max` characters of `alphabet`, the empty one
/// included.
fn every_text(alphabet: &[char], max: usize) -> Vec<String> {
    let mut all = vec![String::new()];
    let mut last = vec![String::new()];
    for _ in 0..max {
        last = last
            .iter()
            .flat_map(|text| {
                alphabet.iter().map(move |&c| {
                    let mut longer = text.clone();
                    longer.push(c);
                    longer
                })
            })
            .collect();
        all.extend(last.iter().cloned());
    }
    all
}

#[test]
fn a_steam_exe_round_trips_as_its_canonical_text() -> TestResult {
    for (text, canonical) in [
        ("steam:440", "steam:440"),
        ("STEAM:440", "steam:440"),
        ("Steam:0440", "steam:440"),
        ("steam:4294967295", "steam:4294967295"),
    ] {
        let mut table = Table::new();
        table.push(Rule::new(Exe::new(text)?));
        assert_round_trips(&table, text);
        assert_eq!(table.to_string(), format!("exe={canonical}"));
    }
    Ok(())
}

#[test]
fn every_short_text_round_trips_in_every_field() -> TestResult {
    // Each text in every field of user data that takes it: the exe, a
    // fingerprint, both sides of a DLL override, an argument, a debug
    // channel, an env name and an env value.
    for text in every_text(ALPHABET, 3) {
        let mut rule = Rule::new(Exe::new(format!("x{text}"))?)
            .with_company(Fingerprint::new(text.clone())?)
            .with_dll_override(DllOverride::new(format!("{text}={text}"))?);
        if let Ok(exe) = Exe::new(text.clone()) {
            rule.set(Key::Exe, exe.as_str())?;
        }
        if let Ok(argument) = Argument::new(text.clone()) {
            rule = rule
                .with_argument(argument.clone())
                .with_arguments_override([argument]);
        }
        if let Ok(channel) = DebugChannel::new(text.clone()) {
            rule = rule.with_debug_channel(channel);
        }
        if let Ok(name) = EnvName::new(text.clone()) {
            rule = rule.with_env(EnvEntry::new(name, text.clone())?);
        }
        rule = rule.with_env(EnvEntry::new(EnvName::new("V")?, text.clone())?);
        // Twice, so that the first record is followed by a line break.
        // `str::lines` drops a CR right before a LF, so a CR that the
        // escapes let through at the end of a record would only be lost
        // there, not at the end of the text.
        let mut table = Table::new();
        table.push(rule.clone());
        table.push(rule);
        assert_round_trips(&table, &format!("{text:?}"));
    }
    Ok(())
}

#[test]
fn what_lenient_parsing_keeps_of_any_short_text_round_trips() {
    // Raw wire text, escapes and bare `%` included, with keys and words in
    // any case: whatever lenient parsing makes of it is a table that writes
    // itself as clean text, which strict parsing reads back as the same
    // rules.
    let alphabet = ['%', '0', '2', '5', 'B', 'c', 'F', 'z', ';', '=', 'é'];
    for text in every_text(&alphabet, 4) {
        for line in [
            format!("exe={text}"),
            format!("exe=x;company={text};arguments={text};env={text}={text};dll_overrides={text}"),
            format!("exe=x;debug_channels={text};arguments_override={text};dxgi={text}"),
            format!(
                "EXE=x;Company={text};DXGI=WineD3D;Dpi_Aware=TRUE;GCMouse=Default;ENV={text}=1"
            ),
            format!("Exe={text};D3D9={text};Metal_HUD=False;Arguments_Override={text}"),
        ] {
            let (table, _) = Table::parse_lenient(&line);
            let written = table.to_string();
            let reread = Table::parse_strict(&written);
            assert_eq!(
                reread.as_ref().map(Table::rules),
                Ok(table.rules()),
                "{line:?} wrote {written:?}"
            );
        }
    }
}
