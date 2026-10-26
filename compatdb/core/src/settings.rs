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
//! `arguments_override` in a rule for every process.

use crate::{
    Argument, DEFAULT_VALUE, DebugChannel, DllOverride, EnvEntry, EnvName, Exe, FieldError,
    Fingerprint, Key, Rule, Scalar, ScalarValue, Setting, Severity,
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
    /// built-in rules then decide as if there were no value. An entry of
    /// `arguments_override`, or an `exe` of `*` in a rule that has one, is
    /// [`FieldError::ArgumentsOverrideOnAny`]: for every process it would
    /// take away the arguments the built-in rules give single programs.
    /// Make the rule with its `exe` first.
    pub fn set(&mut self, key: Key, value: &str) -> Result<(), FieldError> {
        if let Some(scalar) = key.scalar() {
            let setting = scalar_setting(scalar, value)?;
            self.put_scalar(scalar, setting);
            return Ok(());
        }
        match key {
            Key::Exe => {
                let exe = Exe::new(value)?;
                if exe.is_any() && self.arguments_override.is_some() {
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
                self.refuse_override_on_any()?;
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
    /// [`FieldError::NotAList`] for a key that holds one value, and the
    /// first error [`Rule::set`] gives for one of the values.
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
            Key::ArgumentsOverride => {
                rule.refuse_override_on_any()?;
                rule.arguments_override = Some(Vec::new());
            }
            _ => return Err(FieldError::NotAList(key)),
        }
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
    /// [`FieldError::RetiredEnv`] for a variable that a setting replaced
    /// (see [`retired_env`]), and the errors of [`EnvName::new`] and
    /// [`EnvEntry::new`].
    pub fn set_env(&mut self, name: &str, value: &str) -> Result<(), FieldError> {
        if let Some(key) = retired_env(name) {
            return Err(FieldError::RetiredEnv {
                name: name.to_owned(),
                key,
            });
        }
        let entry = EnvEntry::new(EnvName::new(name)?, value)?;
        self.env.push(entry);
        Ok(())
    }

    /// [`FieldError::ArgumentsOverrideOnAny`] when the rule matches every
    /// process by its `exe`.
    const fn refuse_override_on_any(&self) -> Result<(), FieldError> {
        if self.exe.is_any() {
            Err(FieldError::ArgumentsOverrideOnAny)
        } else {
            Ok(())
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
