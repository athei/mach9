//! The settings layer: how a launcher fills a [`Rule`] from its own
//! configuration, key by key and value by value as text, without knowing
//! any key itself.
//!
//! A launcher reads the name of each setting with [`Key::from_setting`],
//! branches on [`Key::kind`] and hands the value to [`Rule::set`] (one value
//! of a matcher or a scalar, or one entry of a list), [`Rule::set_list`] (a
//! whole list) or [`Rule::set_env`] (one environment entry). Every error is
//! a [`FieldError`] whose `Display` is complete enough to print as it is,
//! and whose [`FieldError::severity`] says whether the launcher warns and
//! goes on or refuses the settings.
//!
//! The policy for settings that older launchers wrote lives here too: the
//! retired keys, values and environment variables, and the refusal of
//! `arguments_override` in a rule for every process
//! ([`Key::allowed_on_any`]). So does what a launcher needs to check its
//! configuration before it builds a rule: which keys a later layer of it
//! replaces ([`Key::replaces_on_merge`]) and which environment names a rule
//! refuses ([`check_env_name`]).

use crate::{
    Argument, DEFAULT_VALUE, DebugChannel, DllOverride, EnvEntry, EnvName, Exe, FieldError,
    Fingerprint, Key, KeyKind, Rule, Scalar, ScalarValue, Setting, Severity,
};

/// The keys that are no longer read, each with why and what to do instead
/// (the clause that follows the key in the message) and how a launcher
/// treats it.
const RETIRED_KEYS: &[(&str, &str, Severity)] = &[
    (
        "x87_sidecar",
        "is no longer read: this Wine runs x87 code in FEX and has no x87 sidecar; remove it",
        Severity::Warning,
    ),
    (
        "cmdline_append",
        "is no longer read; use arguments",
        Severity::Error,
    ),
];

/// The values that are no longer read, each with its key, why and what to
/// do instead (the clause that follows the value in the message) and how a
/// launcher treats it.
const RETIRED_VALUES: &[(Key, &str, &str, Severity)] = &[(
    Key::Dxgi,
    "gptk",
    "asks for Apple's D3DMetal, which this Wine does not have, so it is ignored as if it were \
     not there: without another dxgi value the built-in rules decide, which give DXMT unless \
     they pin the program to another tree; remove it",
    Severity::Warning,
)];

/// The environment variables that a setting replaced, each with the key of
/// that setting.
const RETIRED_ENV: &[(&str, Key)] = &[
    ("WINEDEBUG", Key::DebugChannels),
    ("WINEDLLOVERRIDES", Key::DllOverrides),
    ("WINE_CMDLINE_APPEND", Key::Arguments),
    ("WINE_MAC_OPENGL", Key::OpenGl),
    ("WINE_FORCE_NX", Key::ForceNx),
];

/// The setting names that are no longer read, each with how
/// [`Key::from_setting`] treats it: the names it refuses with
/// [`FieldError::RetiredKey`] and that severity.
pub fn retired_keys() -> impl Iterator<Item = (&'static str, Severity)> {
    RETIRED_KEYS
        .iter()
        .map(|&(name, _, severity)| (name, severity))
}

/// The values that are no longer read, each with its key and how
/// [`Rule::set`] treats it: the values it refuses with
/// [`FieldError::RetiredValue`] and that severity.
pub fn retired_values() -> impl Iterator<Item = (Key, &'static str, Severity)> {
    RETIRED_VALUES
        .iter()
        .map(|&(key, value, _, severity)| (key, value, severity))
}

/// The environment variables that a setting replaced, each with the key of
/// that setting: the names for which [`retired_env`] gives that key and
/// [`check_env_name`] refuses.
pub fn retired_envs() -> impl Iterator<Item = (&'static str, Key)> {
    RETIRED_ENV.iter().copied()
}

/// The setting that replaced the environment variable `name`, if one did.
///
/// The name is compared ASCII-case-insensitively: `compatdb.so` merges a
/// rule's env entries into the Windows environment block by name without
/// regard to case, so an entry for `WineDllOverrides` would replace the
/// `WINEDLLOVERRIDES` there. [`Rule::set_env`] refuses such a variable; a
/// launcher can check the environment it sets up itself the same way.
///
/// `WINEDEBUG` is among them: Wine reads it before `compatdb.so` loads, so a
/// rule cannot set it, and `debug_channels` reaches it through
/// [`Table::winedebug`](crate::Table::winedebug).
#[must_use]
pub fn retired_env(name: &str) -> Option<Key> {
    RETIRED_ENV
        .iter()
        .find(|(retired, _)| retired.eq_ignore_ascii_case(name))
        .map(|&(_, key)| key)
}

/// Check `name` as the name of a rule's environment entry, with the errors
/// [`Rule::set_env`] gives for it.
///
/// A launcher runs it on the names of its own configuration, such as an
/// `[env]` table, before it builds a rule from them, or on the environment
/// it starts Wine with.
///
/// # Errors
///
/// [`FieldError::RetiredEnv`] for a variable that a setting replaced (see
/// [`retired_env`]), and the errors of [`EnvName::new`]:
/// [`FieldError::InvalidEnvName`] for an empty name or one with a `=`,
/// [`FieldError::Nul`] for one with a NUL.
pub fn check_env_name(name: &str) -> Result<(), FieldError> {
    env_name(name)?;
    Ok(())
}

/// `name` as the name of a rule's environment entry, or the error of
/// [`check_env_name`].
fn env_name(name: &str) -> Result<EnvName, FieldError> {
    if let Some(key) = retired_env(name) {
        return Err(FieldError::RetiredEnv {
            name: name.to_owned(),
            key,
        });
    }
    EnvName::new(name)
}

/// The variables a launcher removes from the environment it starts Wine
/// with, when it inherited them, so that the launch does not depend on
/// where the launcher itself was started from.
///
/// - `WINEDLLOVERRIDES`: ntdll reads it for every module, so an inherited
///   value would change the load order of the modules the rules'
///   `dll_overrides` leave alone, and a rule's `mscoree,mshtml=default`
///   would hand Mono and Gecko to it.
/// - `WINE_LARGE_ADDRESS_AWARE`: it gives every 32-bit process the 4 GB
///   address space, which `large_address_aware` decides per process.
///
/// `WINE_LARGE_ADDRESS_AWARE` is not retired, so a launcher may still set
/// it for the whole launch on purpose. `WINEDLLOVERRIDES` is (see
/// [`retired_env`]): a rule's `dll_overrides` does its job.
#[must_use]
pub const fn inherited_conflicts() -> &'static [&'static str] {
    &["WINEDLLOVERRIDES", "WINE_LARGE_ADDRESS_AWARE"]
}

impl Key {
    /// The key a launcher's setting called `name` names, compared
    /// ASCII-case-insensitively as [`str::parse`] compares it, which this
    /// is but for the keys that are no longer read.
    ///
    /// `dxgi` and `DXGI` therefore name the same key: a launcher that hands
    /// both to [`Rule::set`] gets the value of the last one for a matcher or
    /// a scalar, and the entries of both for a list.
    ///
    /// # Errors
    ///
    /// [`FieldError::RetiredKey`] for a key that is no longer read, with
    /// its severity: `x87_sidecar` is a warning, since it asks for nothing
    /// this Wine could still do, and `cmdline_append`, which `arguments`
    /// replaced, an error. [`FieldError::UnknownKey`] for any other name
    /// that is no key.
    pub fn from_setting(name: &str) -> Result<Self, FieldError> {
        if let Some(&(_, reason, severity)) = RETIRED_KEYS
            .iter()
            .find(|(retired, ..)| retired.eq_ignore_ascii_case(name))
        {
            return Err(FieldError::RetiredKey {
                key: name.to_owned(),
                reason,
                severity,
            });
        }
        name.parse()
    }

    /// Whether a rule for every process, one whose `exe` is `*`
    /// ([`Exe::Any`]) with or without a fingerprint, may hold this setting.
    ///
    /// It may hold every scalar setting, every list and `env`, but not
    /// `arguments_override`: for every process that would take away the
    /// arguments the built-in rules give single programs. A matcher is not a
    /// setting, so this is `false` for one: `exe` decides whether the rule
    /// is for every process at all, and a fingerprint narrows the processes
    /// it matches, so neither belongs among the settings a launcher offers
    /// for every process.
    ///
    /// [`Rule::set`] and [`Rule::set_list`] refuse a setting for which this
    /// is `false` in a `*` rule, and an `exe` of `*` in a rule that holds
    /// one, with [`FieldError::ArgumentsOverrideOnAny`].
    #[must_use]
    pub const fn allowed_on_any(self) -> bool {
        match self.kind() {
            KeyKind::Matcher => false,
            KeyKind::Scalar(_) | KeyKind::List | KeyKind::Env => {
                !matches!(self, Self::ArgumentsOverride)
            }
        }
    }

    /// Whether a later layer's value for this key takes the place of an
    /// earlier layer's, rather than adding to it, when a launcher builds one
    /// rule from several layers of its configuration, such as a file of
    /// defaults and a file per game laid over it.
    ///
    /// - A matcher or a scalar setting holds one value, so the later layer's
    ///   replaces it: `true`.
    /// - A list adds the later layer's entries after the earlier layer's:
    ///   `false`.
    /// - `arguments_override` is the exception among the lists: `true`. A
    ///   rule has one override, the whole list that replaces the arguments
    ///   of the rules folded before it (see [`Rule::arguments_override`]).
    ///   The entries of two layers added together would be an override
    ///   neither layer wrote, and an empty list in the later layer has to
    ///   clear an earlier layer's override, as `arguments_override=` alone
    ///   clears the arguments on the wire.
    /// - `env` adds the later layer's entries: `false`. The entries fold in
    ///   order, so a later entry for the same variable wins.
    ///
    /// This is only about layers of one rule. Between rules nothing
    /// replaces: every rule that matches a process applies, in the order
    /// [`Table::resolve`](crate::Table::resolve) folds them.
    #[must_use]
    pub const fn replaces_on_merge(self) -> bool {
        match self.kind() {
            KeyKind::Matcher | KeyKind::Scalar(_) => true,
            KeyKind::List => matches!(self, Self::ArgumentsOverride),
            KeyKind::Env => false,
        }
    }
}

/// Whether a `*` rule refuses the field `key`: a setting that
/// [`Key::allowed_on_any`] does not allow. A matcher is not a setting, and a
/// `*` rule takes the fingerprints.
const fn refused_on_any(key: Key) -> bool {
    !matches!(key.kind(), KeyKind::Matcher) && !key.allowed_on_any()
}

impl Rule {
    /// Set the field `key` from `value`, as a launcher's setting gives it:
    /// the value of a matcher or a scalar setting, or one more entry of a
    /// list. For a scalar, `default` in any case resets the setting (see
    /// [`Setting::Reset`]); for `env`, `value` is `NAME=value`, as for
    /// [`Rule::set_env`]. A `debug_channels` value is split at each `,` into
    /// entries of their own, as `WINEDEBUG` reads it, so that
    /// [`Table::winedebug`](crate::Table::winedebug) scopes each of them to
    /// the rule's `exe`; an empty entry, `+relay,` included, is refused.
    ///
    /// On an error the rule is left as it was.
    ///
    /// # Errors
    ///
    /// The [`FieldError`] of a value the key does not take: a word the
    /// setting does not know, a part that is empty or holds a NUL, and the
    /// errors of [`Rule::set_env`]. A retired value is
    /// [`FieldError::RetiredValue`]: `dxgi` `gptk`, a warning, since the
    /// built-in rules then decide as if there were no value. A setting that
    /// [`Key::allowed_on_any`] does not allow, `arguments_override`, in a
    /// rule whose `exe` is `*`, or an `exe` of `*` in a rule that holds one,
    /// is [`FieldError::ArgumentsOverrideOnAny`]: for every process it would
    /// take away the arguments the built-in rules give single programs.
    /// Make the rule with its `exe` first.
    pub fn set(&mut self, key: Key, value: &str) -> Result<(), FieldError> {
        self.refuse_on_any(key)?;
        if let Some(scalar) = key.scalar() {
            let setting = scalar_setting(scalar, value)?;
            self.put_scalar(scalar, setting);
            return Ok(());
        }
        match key {
            Key::Exe => {
                let exe = Exe::new(value)?;
                if exe.is_any()
                    && Key::ALL
                        .iter()
                        .any(|&key| refused_on_any(key) && self.holds(key))
                {
                    return Err(FieldError::ArgumentsOverrideOnAny);
                }
                self.exe = exe;
            }
            Key::Company => self.company = Some(Fingerprint::new(value)?),
            Key::Product => self.product = Some(Fingerprint::new(value)?),
            Key::OriginalFilename => self.original_filename = Some(Fingerprint::new(value)?),
            Key::DllOverrides => self.dll_overrides.push(DllOverride::new(value)?),
            Key::Arguments => self.arguments.push(Argument::new(value)?),
            Key::ArgumentsOverride => {
                let argument = Argument::new(value)?;
                self.arguments_override
                    .get_or_insert_default()
                    .push(argument);
            }
            Key::DebugChannels => {
                let channels = value
                    .split(',')
                    .map(DebugChannel::new)
                    .collect::<Result<Vec<_>, _>>()?;
                self.debug_channels.extend(channels);
            }
            Key::Env => {
                let (name, value) = value.split_once('=').unwrap_or((value, ""));
                self.set_env(name, value)?;
            }
            // The scalar settings returned above.
            Key::Dxgi
            | Key::D3d9
            | Key::Wined3d
            | Key::OpenGl
            | Key::DpiAware
            | Key::LargeAddressAware
            | Key::Gcmouse
            | Key::ForceNx
            | Key::MetalHud => {}
        }
        Ok(())
    }

    /// Make the list `key` hold `values`, each read as [`Rule::set`] reads
    /// one entry, in place of the entries it had. For `arguments_override`
    /// that sets the override, and no values clear the arguments of the
    /// rules folded before (see [`Rule::arguments_override`]). For `env`
    /// each value is `NAME=value`.
    ///
    /// On an error the rule is left as it was.
    ///
    /// # Errors
    ///
    /// [`FieldError::NotAList`] for a key that holds one value,
    /// [`FieldError::ArgumentsOverrideOnAny`] as for [`Rule::set`], even
    /// with no values, and the first error [`Rule::set`] gives for one of
    /// the values.
    pub fn set_list<T: AsRef<str>>(
        &mut self,
        key: Key,
        values: impl IntoIterator<Item = T>,
    ) -> Result<(), FieldError> {
        let mut rule = self.clone();
        match key {
            Key::DllOverrides => rule.dll_overrides.clear(),
            Key::Arguments => rule.arguments.clear(),
            Key::DebugChannels => rule.debug_channels.clear(),
            Key::Env => rule.env.clear(),
            Key::ArgumentsOverride => rule.arguments_override = Some(Vec::new()),
            _ => return Err(FieldError::NotAList(key)),
        }
        self.refuse_on_any(key)?;
        for value in values {
            rule.set(key, value.as_ref())?;
        }
        *self = rule;
        Ok(())
    }

    /// Add the environment entry that sets `name` to `value` after the
    /// rule's other entries; an empty `value` removes the variable.
    ///
    /// # Errors
    ///
    /// The errors of [`check_env_name`] for `name`, and of [`EnvEntry::new`]
    /// for `value`.
    pub fn set_env(&mut self, name: &str, value: &str) -> Result<(), FieldError> {
        self.refuse_on_any(Key::Env)?;
        let entry = EnvEntry::new(env_name(name)?, value)?;
        self.env.push(entry);
        Ok(())
    }

    /// [`FieldError::ArgumentsOverrideOnAny`] when the rule matches every
    /// process by its `exe` and refuses the setting `key` there (see
    /// [`Key::allowed_on_any`]).
    const fn refuse_on_any(&self, key: Key) -> Result<(), FieldError> {
        if self.exe.is_any() && refused_on_any(key) {
            Err(FieldError::ArgumentsOverrideOnAny)
        } else {
            Ok(())
        }
    }

    /// Whether the rule holds the setting `key`: a scalar setting set or
    /// reset, a list or `env` with an entry, an `arguments_override` with or
    /// without entries. `false` for a matcher, which is not a setting.
    fn holds(&self, key: Key) -> bool {
        match key {
            Key::DllOverrides => !self.dll_overrides.is_empty(),
            Key::ArgumentsOverride => self.arguments_override.is_some(),
            Key::Arguments => !self.arguments.is_empty(),
            Key::DebugChannels => !self.debug_channels.is_empty(),
            Key::Env => !self.env.is_empty(),
            Key::Exe | Key::Company | Key::Product | Key::OriginalFilename => false,
            Key::Dxgi
            | Key::D3d9
            | Key::Wined3d
            | Key::OpenGl
            | Key::DpiAware
            | Key::LargeAddressAware
            | Key::Gcmouse
            | Key::ForceNx
            | Key::MetalHud => key
                .scalar()
                .is_some_and(|scalar| !self.scalar(scalar).is_unset()),
        }
    }
}

/// What `text` says about the scalar setting `scalar`: `default`, in any
/// case, resets it, and anything else is a value of it, or a retired one.
fn scalar_setting(scalar: Scalar, text: &str) -> Result<Setting<ScalarValue>, FieldError> {
    if text.eq_ignore_ascii_case(DEFAULT_VALUE) {
        return Ok(Setting::Reset);
    }
    if let Some(&(key, _, reason, severity)) = RETIRED_VALUES
        .iter()
        .find(|(key, value, ..)| *key == scalar.key() && value.eq_ignore_ascii_case(text))
    {
        return Err(FieldError::RetiredValue {
            key,
            value: text.to_owned(),
            reason,
            severity,
        });
    }
    scalar.parse_value(text).map(Setting::Set)
}

#[cfg(test)]
mod tests;
