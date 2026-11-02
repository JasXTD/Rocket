# Update experience mockup

Open `preview.html`. This is a design proposal, not the native implementation.
Version 0.89.0, its notes and the history behind the dialog are sample data.

The approved design is now implemented in Rust for 0.88.1. Native acceptance and
compact-window captures are in `tools/maintenance/update-ui-20261101`; this HTML
remains an inert design preview.

The three main views are a nonblocking startup notice, a focused checking state,
and the available-release review. The selector also covers download, verification,
restart readiness, current version, connection error, no public release and cancel.
The Compact window and Long notes controls exercise constrained layouts.

Download/retry buttons run local simulations. Cancellation clears their pending
transitions. Install and restart displays a preview confirmation; it does not
restart Rocket. The GitHub source link is an ordinary external link. No API or
download request runs automatically, and the page uses no external libraries.

For a native implementation, retain the existing once-per-launch check, manual
Help recheck, explicit restart and validation/recovery behavior. Show real release
notes, never the sample bullets. Progress is phase-based; this proposal does not
invent a percentage or require a byte-progress backend. A dismissed launch notice
leaves an Update available entry in the status bar.

Browser verification and screenshots are recorded in
`tools/maintenance/update-ux-mockup-20261101`. No Rust, release, version or backend
changes were made for this design pass.
