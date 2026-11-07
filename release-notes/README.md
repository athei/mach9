# Release notes

One Markdown file per release, named after its version and committed with the version bump in `Cargo.toml`:

```
release-notes/1.2.0.md
```

A release candidate, `v1.2.0-rc.1`, uses the file of the release it leads to, `1.2.0.md`.

The file is what the app's update window shows, so keep it short and written for users. The release run refuses a tag without it. `launcher/scripts/appcast.sh` embeds it in the appcast, and Sparkle renders the Markdown, so headings and lists work.

Do not hard-wrap these files: write one paragraph or one bullet per line, with headings and blank lines as usual. The GitHub release body starts with the same text and renders a single newline as a line break.

The GitHub release continues with the components the release was built from. Anything longer, for people reading the release page, is added there by hand in the draft.

Only the file of the release being cut is read. The appcast keeps the notes of the earlier releases it lists, and `appcast.sh` carries them over from there, so this README is never taken for a release's notes.
