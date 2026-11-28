# Gramwave design guide

Gramwave treats a Telegram chat as a source of music and the queue as the listening session. Opening, searching, and sorting playlists preserve the active session. Keep the user's place in the list as it updates. Playback changes through explicit controls and track or queue actions.

## Visual language

Use [the shared design code](src/design.rs) for the light and dark palettes, typography, spacing, icons, and controls. Most surfaces use neutral colors. The current theme supplies a raised neutral for fields and hover, a green accent for primary playback, and a softer green fill for active choices. Use the round accent button for the principal play or pause action and the quieter icon buttons for secondary actions. Give icon-only controls translated accessible names.

- **Hover** marks the whole interactive row with a neutral gray fill. Use the same color family for a chat in Library and a track in Tracks. Keep the surface quiet when the pointer leaves.
- **Open chat** uses a green fill in Library. **Current track** has its own playback highlight, with a leading accent and a playback indicator in Tracks. These states stay legible while the pointer or focus moves.
- **Track selection** uses the shared selection treatment, including when a selected row is also current. **Keyboard focus** has a visible outline and can coexist with hover, selection, and playback.
- **Menu choices** use the width of the menu content for the fill, focus outline, and hit target. Keep labels left-aligned and adjacent rows flush. Use the shared menu row spacing and use separators between distinct groups. The playlist filters are compact buttons in the toolbar and full-width rows in the overflow menu. Both use the Default audio filters colors from Settings with plain labels. An enabled filter has accent text and a green fill at rest. Hover and press give it a gray fill. A disabled filter uses the panel fill at rest and green on hover or press. Never use a checkmark inside a circle. In split rows such as **Play next** with a reverse-order action, keep the secondary button separately clickable inside the full-width row.

Keep list geometry predictable. Artwork and the title lead the row. Artist and duration are secondary. Reserve space for row actions and the scrollbar so they stay reachable alongside long metadata. Use shared text styles, translate interface labels with `tr`, and preserve original titles and other user content. See the [playlist row and menu tests](src/screens/tests/playlist.rs) and [window layout tests](src/screens/tests/window.rs).

## Navigation and playback

- **Library, Tracks, and Queue** are the same workspace whether docked side by side or shown as tabs. Library starts with pinned chats. **Find a chat** opens search, with recent chats available before typing. Escape from chat search returns to the pinned view. Opening a chat takes the user to its tracks.
- **Search this playlist** narrows the visible tracks. Activating a matching row starts a new queue from that track in the source playlist. **Play all** starts the playlist in the selected sort order. **Play shuffle** starts with tracks from different parts of the playlist. Both use its media filters even when the text query has no matches. The Music, Audio files, and Voice messages filters are independent. Preserve their choices as they move between toolbar and menu. Enter and Space toggle a focused filter. The overflow menu stays open while switching filters so several can be changed in one visit.
- **Click or Enter** activates an ordinary track row. The ellipsis and right-click open actions for the row or selection. Keep the row and ellipsis as separate hit targets. In lists, Up and Down move focus, Left and Right move between panes, and Tab reaches row actions. Focus movement reveals the destination and leaves playback as it was. Track menus keep navigation inside the menu and return focus to their opener on Escape. Space toggles playback during browsing and yields to text entry and track menus.
- Route playback through the shared application commands so the same action means the same thing in the window, tray, and media controls. The window can be reopened around the active session, and after a restart the saved session waits for Play.

See the [keyboard navigation tests](src/screens/tests/navigation.rs) and [playlist interaction tests](src/screens/tests/playlist.rs).

## Queue and direct manipulation

Tracks is the source list. Queue shows the session around the current occurrence, including playback history and upcoming tracks. A track can appear more than once. Play, selection, reordering, and removal in Queue act on those individual occurrences.

- Keep playlist sort and playback order separate. **Play next** inserts after the current track and **Add to queue** puts tracks at the end. Turning shuffle off restores the original order around the current track and puts pending Play next tracks immediately after it. Turning shuffle on starts a new shuffled pass with the current track first. A group follows the displayed order unless the user chooses the adjacent reverse-order action.
- Track selection gives the user a way to act on a group while listening. Shift extends a range. Ctrl can pick separate tracks in Tracks, while Queue keeps a contiguous range for block edits. Modifier gestures and opening a row menu preserve playback. Drag from a queue row's artwork to select a range or from its body to preview a new order. Reordering commits on drop. Escape cancels the drag.
- Follow the current track until the user scrolls away. Then preserve their reading position through track changes within that queue and offer **Back to current track**. A newly built queue returns to following its current track. Switching shuffle returns to the current row at the top, including during manual browsing. Loading rows before the current track keeps it at the top while following. Preserve the visible playlist anchor when refreshing or changing order, using the current track where appropriate.
- Fetch more queue rows when at most 24 loaded tracks remain ahead of the current track or the top of the list during downward browsing. Keep the playback buffer ready with the queue panel closed.
- Show **Undo** beside the result of a queue removal, including separate notices for independent edits. Ask for confirmation when removing downloaded copies. Describe the action in terms of the queue or local downloads and keep the Telegram source read-only.

See the [queue interaction tests](src/screens/tests/queue.rs).

## Resize and feedback

Keep playback usable while the workspace changes shape. Width moves secondary actions into overflow and side panels into tabs. Height compresses headings and artwork before the track area gives way to the mini-player. The smallest layout keeps Previous, Play/Pause, and Next. Transport hides the volume slider, reverse, shuffle, repeat, then mute as space runs out. Use the existing transitions and keep visible controls operable throughout.

A seek drag previews the position and commits on release. Clicking the timeline or using it with the keyboard also seeks. The time readout has a separate target for switching between elapsed and remaining time.

Show feedback where the action happened. Distinguish loading, no matching tracks, all filters off, and a chat with no music. Keep progress near the relevant playback or download control and put a retry near a recoverable error. When resizing or loading changes presentation, preserve the user's current task and make the next action clear.

Check behavior with `cargo run --locked --no-default-features -- --demo` and add `--light` for the light theme. Compare wide, narrow, and short windows. Exercise mouse and keyboard, including empty space within a row, overflow actions, and transitions between layouts. See the [transport tests](src/screens/tests/transport.rs) and [verification commands](ARCHITECTURE.md#verification).
