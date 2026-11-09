# Share sheet: brief

Outbound share of text, URLs, files and images, and inbound share into the app.

## Options

- Outbound: port share_plus through objc2 (UIActivityViewController, NSSharingService) and Intent chooser via JNI
- Inbound: iOS share extension with App Group handoff; Android ACTION_SEND filters
- Reference only: listen_receive_sharing_intent, react-native-share

## Recommended path

Port the share_plus approach for outbound. Port the listen_receive_sharing_intent design for inbound.

## Edge cases to design for

- Cancelled sheet is not an error: map it separately
- iPad needs a popover anchor, or presenting crashes
- Android file:// is rejected; share through FileProvider content URIs with read grants
- Large files: copy to a temp file, share by URI
- Cold start from a share can deliver MAIN instead of SEND

## Research status

Research done 2026-11-09 by web search. Versions and licences are unverified until checked on the registry before pinning. Full catalogue: the research notes for the SuperNSH library set.
