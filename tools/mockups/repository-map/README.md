# Repository Map design preview

Open `preview.html` in a current browser. The preview uses Rocket's existing navy
palette, cyan selection, Segoe UI typography, compact controls and desktop shell.
It has no package dependencies, external assets or network calls.

All repository records, people, counts, commits and connections are **synthetic**.
Existing Rocket crate names are used only to make the design familiar. This does
not scan Rocket's source, calculate actual dependencies or modify a repository.
No Rust UI or backend implementation is included.

## Explore

- **Code:** choose a snapshot, search crates, select dependencies/consumers, expand
  rocket-ui into files, and open related history or contributors. File links are
  illustrative import relationships, not a promise of language coverage.
- **Git history:** explore parent/child connections, all branches or one branch's
  reachable history, merge context, authors, changed files and links back to Code.
  Changing the view scope never changes the checkout.
- **People:** explore contributors by changed area or shared files. Inspect sample
  paths and commits, including Unicode names. Shared changes are evidence of
  overlap, not proof of direct collaboration or a productivity measure.

Select nodes to inspect them. Drag empty canvas space to pan, scroll to zoom,
or use the zoom, Focus and Fit controls. Focus centers the selection at a readable
scale; Fit shows the whole graph. Small windows start with a readable focus rather
than shrinking every label. Search also brings matching nodes into view.
A focused canvas supports arrow-key panning,
`+`/`-` zoom and `F` to fit. Tab to a node and press Enter/Space to select it.
Left/Right, Home and End navigate the three view tabs. Escape closes details or
the evidence dialog. Plain text filters retain connected context on the canvas.

## Review states and window sizes

The designer strip above the app switches between Ready, Loading, Empty
repository, Read error and Stopped states. Loading stays visible until Stop or
Finish simulation is selected. Refresh/Retry simulate a short read; Stop cancels
that pending result and keeps the previous map available. Nothing reads Git.

The window selector constrains the app to 780 or 480 pixels, or fits the browser.
At compact widths, the details panel becomes a dismissible drawer. The sidebar
collapses to icons and then hides at the smallest sizes; filters wrap. At small
sizes, use zoom/pan to inspect a large graph rather than shrinking all text into
a fixed desktop screenshot.

`preview.html?embed=1` hides the designer strip for viewport tests and captures.
No state is saved outside this page; Reset restores the initial sample.

## Implementation boundary

This preview is a design artifact. A later native implementation still needs
bounded/cancellable data collection, cache invalidation, language coverage,
rename and identity semantics, graph size limits, accessible native controls and
testing against temporary repositories. Code analysis, Git ancestry and People
aggregation should remain separate data sources with explicit provenance.
