# Brainmap Decision Engine Frontend Prompt

You are Elite Frontend UI/UX developer.

Build the frontend for the Brainmap Decision Engine read-only web UI.
attached image

I will provide a reference image. Treat the image as the primary visual target. Recreate the interaction model, layout, mood, and information architecture, but implement it as a real maintainable frontend, not a static screenshot.

The product is:

Brainmap Decision Engine Explorer

It is a read-only local web UI for exploring a personal decision engine. The UI must feel modern, innovative, premium, fast, and secure. It should visually communicate that Brainmap is not a knowledge base but a decision-policy map that helps agents decide like the user.

Do not ask clarifying questions. Make strong defaults. Research latest stable open-source frontend libraries before final choices. Use secure, actively maintained, permissively licensed dependencies only.

---

## 1. Visual reference

Use the attached image as the design reference.

The image shows:

- a dark futuristic dashboard
- browser-like app shell
- left sidebar with Brainmap sections
- top search/status bar
- read-only status chip
- shadow mode chip
- autopilot conservative chip
- large semi-3D brain visualization
- labeled brain sections:
  - Decision Identity
  - Tradeoff Models
  - Restrictions
  - Choice Patterns
  - Question Triggers
  - Calibration
  - Examples
- selected brain region exploding outward into a graph/map
- node clusters
- policy cards
- rule cards
- bottom insight cards
- premium SaaS-style polish
- subtle neon cyan/violet/teal/gold accents
- glassmorphism panels
- no clutter

Implement a working UI that follows this direction.

Do not use the reference image as a full-screen background. Build the UI with real components, SVG/canvas/HTML/CSS, reusable data models, and interaction states.

You may use the image only as a design reference. If you create any image asset from it, document why and ensure the UI still works without it.

---

## 2. Tech stack

Research current best options and choose a secure, modern open-source stack.

Preferred default unless research shows a better choice:

- Vite
- React
- TypeScript
- Tailwind CSS
- shadcn/ui or Radix primitives where appropriate
- Framer Motion or Motion One for animations
- D3, React Flow, Cytoscape.js, or custom SVG for graph visualization
- Vitest
- Testing Library
- Playwright for end-to-end tests
- ESLint
- Prettier

Do not use:

- external CDNs
- remote fonts
- analytics
- telemetry
- write APIs
- cloud services
- paid/proprietary UI kits
- abandoned graph libraries
- GPL/AGPL dependencies unless explicitly isolated and approved

All assets must be local.

If using icons, use a permissively licensed open-source icon set such as Lucide or implement inline SVG icons.

Document dependency decisions in:

```text
docs/frontend-dependency-decisions.md
```

---

## 3. Product behavior

This frontend is read-only.

It must not allow editing, deleting, writing, approving, exporting, importing, or changing autopilot state.

It may display:

- policies
- tradeoff rules
- hard restrictions
- approval rules
- ask triggers
- examples
- counterexamples
- calibration score
- confidence
- coverage
- stale policies
- recent decisions
- graph relationships
- read-only status
- shadow/autopilot status

The UI should make it obvious that it is an explorer, not a control panel.

---

## 4. App shell

Implement an app shell similar to the reference image.

Layout:

```text
Brainmap Explorer
├── top browser/app bar
├── left sidebar
├── central brain visualization
├── exploded graph/map area
├── policy/rule cards
└── insights panel
```

Top bar must include:

- app title: `Brainmap`
- subtitle: `Decision Engine Explorer`
- global search field
- status chip: `Read-only`
- status chip: `Shadow Mode`
- status chip: `Autopilot: Conservative`
- help icon
- notification icon, read-only placeholder
- avatar initials placeholder

Left sidebar sections:

- Decision Identity
- Tradeoff Models
- Restrictions
- Choice Patterns
- Question Triggers
- Calibration
- Examples

Each sidebar item has:

- icon
- title
- short description
- active state
- hover state
- keyboard focus state

Bottom sidebar card:

- title: `Read-only Explorer`
- text explaining that this UI cannot mutate the Brainmap Decision Engine
- link/button-style affordance: `Learn more`, but read-only

---

## 5. Brain visualization

Implement a stylized brain map component.

It does not need to be anatomically perfect, but it must feel like a premium cognitive atlas.

Each brain section should be a clickable/selectable region.

Brain sections:

1. Decision Identity
2. Tradeoff Models
3. Restrictions
4. Choice Patterns
5. Question Triggers
6. Calibration
7. Examples

Each section must have:

- label
- icon
- distinct accent color
- active state
- hover state
- keyboard selectable state
- description
- associated node cluster

Suggested colors:

- Decision Identity: teal/green
- Tradeoff Models: violet/purple
- Restrictions: amber/gold
- Choice Patterns: cyan/blue
- Question Triggers: magenta/purple
- Calibration: mint/teal
- Examples: blue

Implementation options:

- SVG blob sections
- layered CSS/SVG shapes
- canvas
- simple 3D-ish SVG with gradients
- hybrid HTML/SVG

Prefer SVG for testability and accessibility.

The brain should have:

- subtle glow
- section boundaries
- animated active pulse
- particle/connection accents
- reduced-motion fallback

---

## 6. Exploded graph interaction

When a brain section is selected, show an exploded graph/map view to the right.

The selected brain region should visually emit or connect to the graph.

Interaction:

- default selected section: `Tradeoff Models`
- clicking a section changes graph clusters and cards
- selected section animates outward into nodes
- graph nodes fade/slide into place
- related policy cards update
- search filters graph/cards
- sidebar active state updates
- URL route or query param may reflect selected section if useful

Graph view should contain:

- central hub node
- several clusters
- labeled nodes
- curved or straight edges
- floating policy/rule cards
- node hover tooltip
- selected node details panel or card highlight

Example for `Tradeoff Models`:

Central hub:

- Tradeoff Models

Clusters:

- Core Tradeoff Principles
- Risk & Downside Management
- Effort & Resource Tradeoffs
- Value Alignment Tradeoffs

Nodes:

- Impact vs Effort
- Short-term vs Long-term
- Optionality Value
- Reversibility Premium
- Downside Protection
- Tail Risk Aversion
- Regret Minimization
- Effort Justification
- Resource Allocation
- Attention Economy
- Values Alignment
- Mission Alignment
- Integrity Check

Cards:

- Policy: Long-term Compounding Bias
- Policy: Optionality Preservation
- Rule: Avoid Severe Irreversible Loss
- Rule: High Effort Requires High Signal
- Policy: Values First Tie-breaker

For other sections, create plausible fixture data.

---

## 7. Data model

Create a typed frontend data model.

Types:

```ts
type BrainSectionId =
  | "decision-identity"
  | "tradeoff-models"
  | "restrictions"
  | "choice-patterns"
  | "question-triggers"
  | "calibration"
  | "examples";

type BrainSection = {
  id: BrainSectionId;
  label: string;
  description: string;
  icon: string;
  color: string;
  nodeCount: number;
  policyCount: number;
};

type GraphNode = {
  id: string;
  label: string;
  kind:
    | "hub"
    | "policy"
    | "rule"
    | "tradeoff"
    | "restriction"
    | "example"
    | "question"
    | "calibration";
  sectionId: BrainSectionId;
  confidence?: number;
  status?: "seed" | "tested" | "reliable" | "stale" | "contradicted";
  x?: number;
  y?: number;
};

type GraphEdge = {
  id: string;
  source: string;
  target: string;
  kind:
    | "related"
    | "tradeoff"
    | "restriction"
    | "approval"
    | "example-of"
    | "counterexample-of"
    | "contradicts";
  weight?: number;
};

type PolicyCard = {
  id: string;
  title: string;
  kind: "policy" | "rule" | "restriction" | "ask-trigger" | "example";
  summary: string;
  sectionId: BrainSectionId;
  confidence: number;
  status: "seed" | "tested" | "reliable" | "stale" | "contradicted";
  tags: string[];
  links: string[];
};

type EngineInsight = {
  id: string;
  label: string;
  value: string;
  delta?: string;
  tone: "neutral" | "good" | "warning" | "critical";
};
```

Create fixture data in:

```text
src/data/demoBrainmap.ts
```

Later this can be replaced by a read-only API from the Rust backend.

---

## 8. API layer

Implement a clean read-only API adapter.

For now, support fixture mode.

Structure:

```text
src/
  api/
    brainmapApi.ts
    fixtureBrainmapApi.ts
    types.ts
```

Read-only methods:

```ts
getSections()
getSectionGraph(sectionId)
getPolicyCards(sectionId)
getInsights()
search(query)
getStatus()
```

Do not implement write methods.

If the backend is available later, it can expose:

```text
GET /api/status
GET /api/sections
GET /api/sections/:id/graph
GET /api/sections/:id/cards
GET /api/insights
GET /api/search?q=
```

The frontend must be able to run standalone with fixtures.

---

## 9. Search

Implement global search.

Search across:

- section labels
- node labels
- policy titles
- policy summaries
- tags

Behavior:

- empty search shows selected section
- non-empty search filters cards and graph highlights
- search result count displayed
- keyboard accessible
- no network required in fixture mode
- debounced input

---

## 10. Insights panel

Implement bottom or side insight cards like the reference image.

Cards:

- Total Policies & Rules
- Recent Decisions
- Confidence Trend
- Stale Policies
- Coverage
- Calibration Score

Each card should have:

- label
- large value
- small delta/explanation
- icon
- tone color
- optional mini chart for confidence trend

Use fixture data.

---

## 11. Read-only enforcement

The UI must make read-only state clear.

Requirements:

- `Read-only` chip always visible.
- No edit buttons.
- No delete buttons.
- No save buttons.
- No mutation API calls.
- No forms except search/filter.
- Any action that sounds mutating should show disabled/read-only explanation or not exist.
- Tests must verify there are no write methods or mutation buttons.

---

## 12. Accessibility

Implement:

- semantic landmarks
- keyboard navigation
- focus states
- aria labels for brain sections
- aria labels for graph nodes or fallback list
- reduced-motion support
- sufficient contrast
- no information communicated only by color
- screen-reader friendly selected section summary

Add an accessible fallback panel listing graph nodes and policy cards.

---

## 13. Performance

Target:

- local startup fast
- no external network
- graph with fixture data renders smoothly
- animations under control
- reduced motion support
- no huge dependencies unless justified

Add lightweight performance notes in:

```text
docs/frontend-performance.md
```

Avoid:

- large 3D engines
- WebGL unless strongly justified
- heavy animation frameworks if CSS/SVG is enough
- blocking main thread with expensive layout loops

---

## 14. Styling direction

Follow the reference image.

Visual language:

- dark navy/black background
- subtle gradients
- soft glow accents
- glass panels
- rounded corners
- thin borders
- faint grid/starfield particles
- modern typography
- polished hover states
- elegant icons
- premium SaaS feel
- readable text
- not too noisy

Use design tokens:

```ts
colors:
  background
  panel
  panelElevated
  border
  textPrimary
  textSecondary
  accentCyan
  accentViolet
  accentTeal
  accentAmber
  accentMagenta
  danger
  warning
  success

radii:
  sm
  md
  lg
  xl

shadows:
  glowCyan
  glowViolet
  panel
```

Create:

```text
src/styles/tokens.css
```

or equivalent Tailwind theme extension.

---

## 15. Routing

Implement routes:

```text
/
  redirects to /explorer

/explorer
  default explorer

/explorer/:sectionId
  selected section

/search?q=
  optional, may stay inside explorer
```

If using React Router, keep routing minimal.

---

## 16. Components

Create reusable components:

```text
src/components/
  AppShell.tsx
  TopBar.tsx
  Sidebar.tsx
  StatusChip.tsx
  BrainAtlas.tsx
  BrainSection.tsx
  ExplodedGraph.tsx
  GraphNode.tsx
  GraphEdge.tsx
  PolicyCard.tsx
  InsightCard.tsx
  SearchBox.tsx
  ReadOnlyNotice.tsx
  SectionSummary.tsx
  AccessibleGraphList.tsx
```

Keep components clean and typed.

---

## 17. Testing

Implement tests.

Use Vitest and Testing Library.

Required tests:

1. App renders.
2. Sidebar contains all seven sections.
3. Default selected section is Tradeoff Models.
4. Clicking Decision Identity changes selected section.
5. Clicking Restrictions changes graph/cards.
6. Search filters policy cards.
7. Read-only chip is visible.
8. No edit/delete/save buttons are rendered.
9. API adapter exposes only read methods.
10. Status chips render Shadow Mode and Autopilot Conservative.
11. Brain sections are keyboard focusable/selectable.
12. Reduced motion mode does not break UI.
13. Accessible graph fallback renders node list.
14. Policy cards show confidence/status.
15. Insights panel renders all metrics.
16. No external asset URLs are used in rendered DOM.
17. Snapshot or DOM test for core layout.

Use Playwright if practical.

Playwright E2E:

1. Page loads.
2. Select each brain section.
3. Graph updates.
4. Search works.
5. Read-only UI has no mutation actions.

---

## 18. Security

Requirements:

- No external scripts.
- No external CSS.
- No external fonts.
- No analytics.
- No telemetry.
- No write endpoints.
- No localStorage secrets.
- No eval.
- No dangerouslySetInnerHTML unless sanitized and justified.
- Content rendered from fixture/API must be treated as untrusted.
- Escape/sanitize user-visible dynamic content.

Add:

```text
docs/frontend-security.md
```

---

## 19. Backend integration readiness

Although this frontend can run standalone, prepare it for the Rust Brainmap backend.

Expected future backend:

```text
brainmap web --vault ~/BrainMap --host 127.0.0.1 --port 8777
```

Frontend should support an environment flag:

```text
VITE_BRAINMAP_API_MODE=fixture|http
VITE_BRAINMAP_API_BASE=http://127.0.0.1:8777/api
```

In fixture mode, no network.

In http mode, only GET requests.

If API fails, show a graceful read-only error state.

---

## 20. Deliverables

Create:

- working frontend app
- fixture data
- typed API layer
- interactive brain atlas
- exploded graph/map view
- policy cards
- insight cards
- read-only status UI
- tests
- docs
- build scripts
- README instructions

README must include:

```bash
pnpm install
pnpm dev
pnpm build
pnpm test
pnpm test:e2e
```

or equivalent commands if not using pnpm.

---

## 21. Acceptance criteria

The frontend is acceptable only when:

### Build

1. Dependencies install.
2. TypeScript typecheck passes.
3. Lint passes.
4. Unit tests pass.
5. Production build succeeds.
6. App runs locally without backend in fixture mode.

### Visual/product

7. UI clearly matches the attached reference direction.
8. Dark premium dashboard shell exists.
9. Left sidebar exists.
10. Top search/status bar exists.
11. Read-only chip exists and is always visible.
12. Shadow Mode chip exists.
13. Autopilot Conservative chip exists.
14. Brain visualization has seven labeled sections.
15. Default selected section is Tradeoff Models.
16. Selected brain section visually glows/highlights.
17. Selected section explodes/expands into graph view.
18. Graph has nodes, edges, clusters, and cards.
19. Policy/rule cards appear.
20. Bottom/side insight metrics appear.
21. UI is not a static screenshot.

### Interaction

22. User can select each brain section.
23. Graph updates for each section.
24. Cards update for each section.
25. Search filters/highlights results.
26. Keyboard navigation works for sections.
27. Reduced motion preference is respected.
28. Responsive layout works at common desktop sizes.
29. Graceful empty state exists.
30. Graceful API error state exists.

### Read-only

31. No edit buttons.
32. No delete buttons.
33. No save buttons.
34. No mutation API methods.
35. No POST/PUT/PATCH/DELETE calls.
36. Read-only notice explains constraints.
37. Tests enforce read-only UI.

### Accessibility

38. Semantic landmarks exist.
39. Brain sections have aria labels.
40. Selected section is announced or represented accessibly.
41. Graph has accessible fallback list.
42. Focus states are visible.
43. Contrast is acceptable.
44. No information depends only on color.

### Security

45. No external CDNs.
46. No external fonts.
47. No analytics.
48. No telemetry.
49. No eval.
50. No unsafe HTML rendering unless sanitized and documented.
51. Dynamic text is escaped/safe.
52. No secrets in code or fixtures.

### Integration readiness

53. Fixture API works.
54. HTTP read-only API adapter exists.
55. API mode can switch fixture/http.
56. Backend failure shows read-only error state.
57. Static build can be served by Rust backend later.

### Docs

58. README explains how to run.
59. docs/frontend-dependency-decisions.md exists.
60. docs/frontend-security.md exists.
61. docs/frontend-performance.md exists.
62. docs/ui-reference-notes.md explains how the attached image informed the implementation.

---

## 22. Implementation plan

Use this development loop:

1. Inspect the attached image and write `docs/ui-reference-notes.md`.
2. Research and choose frontend libraries.
3. Create dependency decision doc.
4. Create app skeleton.
5. Implement fixture data and API adapter.
6. Implement app shell.
7. Implement brain atlas.
8. Implement exploded graph.
9. Implement cards and insights.
10. Implement search.
11. Implement accessibility fallback.
12. Implement read-only guardrails.
13. Add tests.
14. Add docs.
15. Run typecheck, lint, tests, build.
16. Fix issues.
17. Final response with commands run and acceptance status.

Do not stop after rough scaffolding. Build a working, polished MVP.

---

## 23. Final response

When done, respond with:

### Built

What was implemented.

### Visual match

How the attached reference image was translated into components.

### Commands run

Exact commands and results.

### Tests

Pass/fail summary.

### Acceptance status

List passed/deferred criteria.

### How to run

Exact local commands.

### Caveats

Honest remaining issues.

