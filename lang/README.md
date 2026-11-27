# Translations

Vynz's source strings are Turkish. Translations are gettext `.po` files:

```
lang/<language>/LC_MESSAGES/vynz.po
```

The same file is used by the Slint interface (`@tr(...)` in `ui/*.slint`, embedded at
build time) and by the Rust code (`tr!` / `trn!` in `src/`, read at runtime by
`src/i18n.rs`).

## Updating after code changes

```sh
cargo install slint-tr-extractor   # once
python3 lang/update.py -v          # adds new strings, drops unused ones, lists missing translations
```

Untranslated strings fall back to Turkish.

## Placeholders

- `{}` is filled in order (`{0}`, `{1}` also work). Keep the same number of `{}` in the translation.
- `{n}` is the count in plural entries (`msgid_plural`, `msgstr[0]` singular, `msgstr[1]` plural).
- Entries with `msgctxt` are the same Turkish word used in a different place.

## Adding a language

Adding a language currently needs a small code change as well (language list in
`src/i18n.rs` and the language picker in `ui/app.slint`). Open an issue, or a pull
request with the new `.po` file, and we'll wire it up.
