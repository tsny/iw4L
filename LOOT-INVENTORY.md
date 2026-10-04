# Loot and grid inventory design

Status: implementation in progress; catalog and grid core complete. Last updated: 2026-10-04.

This is the canonical, living source for the loot and grid-inventory work. It records the intended architecture, package boundaries, current state, and next handoff. Chats are not a source of truth. Session evidence and historical decisions belong in `context/artifacts/2026-10-04-loot-inventory/`; this file is updated when the present design or status changes.

## Session start

A new agent should read, in order:

1. `CONTEXT.md` and `AGENT.md` for repository workflow and publication rules.
2. This file in full.
3. `docs/SIM-STEP.md`, then only the documents named by the selected work package.
4. The loot artifact README and its newest iteration, if the ignored artifact exists on this machine.
5. `git status --short` and the current diff before changing anything.

Work on one package at a time. Update **Current state and next handoff** before ending a session. Put probes, logs, screenshots, and experimental results in the artifact, never in the tracked tree. Do not rewrite closed artifact iterations.

## Goals

The first playable slice provides a Stalker-like backpack: authored items with rectangular footprints, rotation and stacking; an authoritative player inventory; a two-pane player/container UI; world loot bags; and one working consumable. It must preserve multiplayer authority, replay determinism, and content compatibility.

The design must also be productive without installed game data. Catalog validation, grid behavior, and an inventory authoring preview must run on an asset-free macOS checkout. Actual material/model resolution and map acceptance remain game-data checks.

## Non-goals for the first slice

- Weapon instances, attachments, magazines, equipment slots, armor, durability effects, economy, traders, hunger, quests, extraction, and a persistent stash.
- Procedural map placement or changes to retail map files.
- Arbitrary item scripts or client-authoritative inventory mutation.
- Replacing the existing MW2 weapon/loadout inventory.

## Existing seams

- Every authoritative mutation enters `TickInput`, runs through `sim::step`, and leaves through `Snapshot`; authority, prediction, and replay share that funnel.
- `crates/sim/src/item.rs` is a weapon pickup subsystem: it encodes weapon indices, ammo, dual wield, scavenger behavior, automatic touch pickup, and a 16-drop cap. Generic loot does not extend or reinterpret it.
- `SessionContentManifest` already represents match content compatibility and is where the loot-catalog digest belongs.
- `UiLayer::Overlay` is the correct presentation layer. The existing retail menu runtime is useful for ordinary menus, but not for draggable, rotated, multi-cell items.
- Gameplay input and cursor capture are currently coordinated around the console and script menus. Inventory adds a shared modal-input contract in `frame`; it does not create a dependency from `console` to inventory UI.

## Ownership boundaries

| Area | Owns | Must not own |
|---|---|---|
| `crates/inventory` | IDs, item definitions, catalog validation, item instances, grids, placement and transaction rules | Bevy, filesystem policy, networking, rendering, MW2 asset loading |
| `content/loot` | Versioned item and loot-table JSON | Runtime state or game assets |
| `session` | Loading authored packs, resolving asset references, assigning deterministic session IDs, manifest digest, installing runtime facts | Inventory mutation |
| `sim/src/inventory` | Authoritative containers, access grants, revisions, actions, effects, death/drop rules, snapshot semantics | UI state or filesystem reads |
| `net/src/transport/inventory_wire.rs` | Bounded action and revision-sync codecs | Placement policy or authority decisions |
| `crates/inventory_ui` | Overlay, drag ghost, tooltips, local placement preview, pending transaction UX | Committing inventory state |
| `render_anim/src/occupancy/loot.rs` | Posing and lighting world loot containers | Item ownership or transfer rules |
| `inventory_lab` | Asset-free authoring and interaction preview using placeholders | A second gameplay implementation |

Dependency direction is `content → inventory → session/sim → net`, with `inventory_ui` reading the catalog and presented snapshots. Rendering reads presented world-loot semantics. No lower layer imports UI or rendering.

## Durable model

Authored identities and runtime identities are separate:

```rust
ItemKey(String)          // durable, for example "base:bandage"
ItemDefId(u16)           // assigned by sorted key at session install
ItemInstanceId(u64)      // stable while an item moves between containers
ContainerId(u32)         // player backpack, world bag, corpse, later stash

ItemInstance { id, definition, quantity, condition }
Placement { instance, x: u8, y: u8, rotated: bool }
GridContainer { id, width: u8, height: u8, revision: u32, placements }
```

Grid operations are deterministic and total: bounds, overlap, rotation, stack capacity, split quantity, and integer weight are validated before mutation. The grid has no hidden auto-compaction. Auto-placement, when requested, scans row-major with unrotated orientation first. Definitions use integer grams and condition uses a bounded integer scale rather than floating point.

An item instance retains its ID when transferred, dropped, looted, or persisted. Entity numbers are presentation/occupancy identities and are never item identities.

## Authoring contract

Item packs are strict, schema-versioned JSON. Unknown fields fail validation. A representative definition is:

```json
{
  "schema": 1,
  "items": [
    {
      "key": "base:bandage",
      "name": "Bandage",
      "footprint": [1, 1],
      "rotatable": false,
      "max_stack": 4,
      "weight_g": 80,
      "icon": "iw4:material/hud_icon_bandage",
      "world_model": "iw4:xmodel/prop_medical_pack",
      "use": { "kind": "heal", "amount": 25 }
    }
  ]
}
```

Structural validation rejects duplicate or malformed keys, zero or excessive dimensions, invalid stack limits, invalid integer ranges, unknown typed effects, missing loot-table references, and nondeterministic duplicate entries. Asset-free validation checks asset-key syntax. Session installation separately resolves referenced assets and reports missing presentation without pretending structural validation proved availability.

Effects are a closed, versioned enum such as `heal` and later `restore_held_ammo`; authored files do not execute code. Catalog rows are sorted by durable key before assigning `ItemDefId`. The canonical encoding, including schema and gameplay fields, contributes to the session content digest.

The initial authored set is bandage (stacked heal), medkit (large heal), rifle-ammo pouch (typed held-ammo effect), and radio parts (inert transfer/value item). Missing icons/models render as explicit placeholders in the lab; runtime policy for missing resolved assets is decided before world loot lands.

Loot tables are a separate versioned file containing deterministic weighted entries and quantity ranges. Map placement is deferred; the first slice spawns a named table through a debug/authoring command.

## Authority and transactions

Opening a world container is an authority request. The authority verifies lifecycle, range, line of sight, entity/container identity, and access policy, then publishes the granted `ContainerId`. Moving out of range, entity removal, death, or session teardown revokes access.

A drag emits one fixed-size action when dropped; pointer motion is never network traffic. The transfer carries request ID, source and destination IDs and expected revisions, item instance, quantity, destination cell, and orientation. The authority re-derives ownership and source placement, validates both revisions, applies the transfer atomically, increments affected revisions, and emits a reliable accept or typed rejection. Stale revisions never partially apply.

Use, split, merge, drop, and transfer are distinct typed operations even if they share an internal transaction engine. Initially the UI waits for acceptance and marks the instance pending. Optimistic mutation can be added only after rollback semantics exist.

All action variants remain bounded and wire-decodable. Authority limits container dimensions, item counts, action counts, and decoded allocation before accepting peer data.

## Snapshots, privacy, and replay

The authoritative snapshot contains complete inventory semantics so adoption, demos, and replay can restore the world. Per-peer seat filtering retains only the player's containers and currently granted world container; other players' backpacks are private.

Inventory wire sync is revision-based and stateful, like world-object synchronization: unchanged containers cost only bounded revision metadata, changed containers carry a compact canonical row set, and the decoder reconstructs the complete snapshot view. The catalog itself is not sent every frame; peers prove the same digest during match admission.

Inventory actions are recorded in `TickInput`, and resulting state is in `Snapshot`. A replay never reads current disk authoring to decide a historical transfer. Snapshot hashing includes reconstructed inventory semantics in canonical container/instance order.

## World loot and presentation

Generic loot is an aggregate container, not one replicated entity per item. A loot bag or crate owns a `ContainerId` and a world occupancy record containing its entity reference, position/trajectory, and presentation definition. One bag may hold many instances without exhausting entity slots.

The subsystem does not overload weapon `ET_ITEM` indices. The exact entity representation is chosen in the world-loot package after checking entity-kernel and renderer constraints; snapshot semantics remain a distinct `LootWorldContainer` record either way. Rendering resolves only the authored container model and never inspects or mutates contents.

Looking at an accessible bag produces a use hint. Pressing Use requests access; it does not auto-transfer. On death, the first-slice policy is configurable until the owner chooses between retaining match inventory and atomically moving it to one corpse/bag container.

## UI and input

The first overlay has an 8×6 player grid on the left and an opened world container on the right. Dimensions are provisional authoring/rules values, not constants embedded in layout code. Item tiles span their footprint, display stack count and pending state, and expose name, weight, condition, and effect in a tooltip. Dragging shows local valid/invalid placement; `R` rotates when permitted.

`inventory_ui` publishes a generic modal-input claim through `frame`. Console input generation consumes the combined modal state, releases held gameplay buttons, suppresses look/move, and releases the cursor. Escape closes the inventory before opening the pause menu. Keyboard/controller navigation is a later package, but the data model must not depend on mouse coordinates.

The asset-free lab uses the same catalog, grid engine, and UI components with generated placeholder images and an in-process fake transaction adapter. It exists as the maintained authoring preview, not as an alternate authority path.

## Work packages

### Catalog and grid core

Status: complete on 2026-10-04.

Owned paths: new `crates/inventory/`, workspace manifests, and `content/loot/base/`. Deliver strict schema parsing, canonical catalog/digest input, instance/container types, placement/rotation/stack/transfer validation, the initial item pack, and public invariant checks. This package requires no Bevy or game data. Complete when `cargo check -p inventory` and asset-free catalog/invariant checks pass.

### Asset-free authoring tools

Owned paths: new `crates/inventory_lab/` and `xtask/src/loot.rs`, plus minimal workspace/xtask registration. Depends only on catalog/grid core. Deliver `cargo xtask loot validate` and a maintained placeholder preview that exercises the real grid/UI components once they exist. Before UI exists, validate and print a compact catalog/layout report. Complete when a clean checkout can inspect all authored items without `IW4L_GAMES`.

### Simulation integration

Owned paths: new `crates/sim/src/inventory/` plus narrow hooks in input, step, world/frame, adoption, and snapshot metadata. Depends on catalog/grid core. Deliver player containers, deterministic instance allocation, revisioned transactions, typed rejection/accept events, canonical snapshots, teardown, and replay-safe cloning. No world entities or persistence. Complete when an artifact probe demonstrates accepted, overlapping, out-of-bounds, and stale-revision moves through `sim::step` and snapshot adoption.

### Network transport

Owned paths: new `crates/net/src/transport/inventory_wire.rs` plus narrow registrations in action/meta/frame codecs and seat filtering. Depends on simulation integration. Deliver bounded action codecs, revision sync, private projection, decoder reconstruction, and authoritative hash coverage. Complete when wire round trips and malformed-input evidence are recorded and existing protocol callers build.

### Inventory overlay

Owned paths: new `crates/inventory_ui/`, the generic modal contract in `frame`, and narrow plugin/input registration. Depends on catalog/grid core and presented simulation state; it may begin against the lab adapter before network transport is finished. Deliver two grids, drag/rotate, tooltip, pending UX, cursor/input capture, teardown, and placeholder icons. Complete when the same UI works in the asset-free lab and emits one transaction per completed drop.

### World loot containers

Owned paths: a focused sim world-loot module, use-target integration, and `render_anim/src/occupancy/loot.rs`; session installs presentation facts. Depends on simulation and transport. Deliver a debug spawn command, one aggregate bag model, access grant/revoke, use hint, and player↔bag transfers. Complete when a two-client run proves distance/access checks and private contents, with rendering evidence deferred if game data is unavailable.

### Basic item effects

Owned paths: typed effect application inside sim and corresponding reliable UI feedback. Depends on the authoritative inventory. Deliver bandage/medkit healing and held-ammo restoration with lifecycle, health/ammo cap, consumption, interruption, and duplicate-request rules. Complete when replaying the same action stream produces the same inventory and player state.

### Persistence and extraction

Deferred until the match-scoped slice is stable. Do not store custom loot in the retail structured-data buffer. Design a schema-stamped loot account snapshot with atomic save and conflict detection, then extend account admission/ack transport deliberately. Stash and extraction rules are a separate owner decision and work package.

## Verification policy

This repository does not keep ordinary unit/probe tests outside owner-approved scenarios. Permanent catalog validation and invariant checking belong in the maintained authoring command. Disposable probes and their outputs live in the loot artifact. Add an `approved_tests` scenario only after the owner approves it by name.

Every package runs the narrowest relevant Cargo checks and records what could not be exercised without game data. Integration packages also check replay/wire determinism and `make publish-check`. Do not claim visual or map behavior from an asset-free preview.

## Current state and next handoff

- Implemented: the pure `inventory` crate; strict schema-1 JSON parsing; canonical sorted IDs and digest bytes; item, instance, placement and container identities; bounded grid validation; deterministic first-fit; rotation; split/merge; atomic revision-checked cross-container transfers; weight calculation; and the four-item base pack.
- Verified: `cargo check --workspace --all-targets`; crate-local Clippy with warnings denied; an asset-free disposable probe covering catalog reordering/digest stability, unknown-field rejection, overlap rejection, rotation, split/merge, cross-container transfer, stale revisions, out-of-bounds atomicity, invariant checks, and weight; the probe was removed after its output was recorded in the artifact.
- Not verified: live inventory UX, world-model availability, map use targeting, multiplayer bandwidth, death policy, or persistence.
- Current blocker: live map/render acceptance needs legally obtained game data. Catalog, grid, authoring tools, simulation, codecs, and placeholder UI are not blocked.
- Next package: **Asset-free authoring tools**. Do not begin simulation integration, persistence, weapon-item bridging, or world rendering first.
- Owner decisions still open: inventory/death retention policy; default backpack dimensions; initial keyboard binding; missing runtime presentation policy; whether rifle ammo targets the held weapon or an authored weapon family.

## Session finish

Before handing off, update this section with implemented facts, verification, gaps, and the exact next package. Add a new artifact iteration with evidence and a mandatory “what was not done” section. State whether scaffolding was removed, retained as a maintained tool, or never added. Leave prior artifact iterations unchanged.
