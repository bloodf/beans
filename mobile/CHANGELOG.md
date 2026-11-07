# Changelog

The phone app's release notes, by the version in `app.config.ts`. A release (a `mobile-vX.Y.Z`
tag) takes a version's section as the notes of its GitHub release, and the Android app shows them
when it offers the update.

## [Unreleased]

- Attention lists what needs you across chats: each coordinator's brief, then the items. Open it
  from the chat list beside Settings while something waits.
- Long-press a brief in Attention and choose Dismiss to take it off the list on every Device; the
  coordinator's next brief shows again. A brief also leaves once every item it is about is resolved.
- Tapping an Attention item that waits on a review opens its chat with that review's read-only
  details on top (approve and reject arrive with the review-queue port).
- A bot resolves an Attention item it alone reported as soon as it sees the work is done, such as a
  run it approved or a pull request that merged, so you no longer clear them by hand.
