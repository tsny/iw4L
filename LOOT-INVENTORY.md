# Loot and grid inventory design

Status: implementation in progress; lightweight native NPC target foundation complete asset-free. Last updated: 2026-10-04.

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

## Debug world spawning and basic pickups

The first authoring surface is the existing console, not a new spawn UI. It already provides parsing, completion, cheat gating, request IDs and feedback, and a later palette can issue the same typed requests. The planned grammar is:

```text
spawn weapon <weapon> [attachment...]
spawn loot <namespace:item> [quantity]
spawn loot-table <namespace:table>
spawn npc target <map-model>
spawn npc sentry|grunt <archetype>
spawn npc bot
```

`spawn loot` means “create one world bag containing this authored item,” not “make the generic item an `ET_ITEM`.” Ordinary enemies are native lightweight actors rather than simulated clients: `target` is the damageable stationary foundation, followed by a stationary `sentry` and mobile `grunt`. `spawn npc bot` remains an optional high-fidelity path for player-compatible opponents. Commands are registered only as their typed backends become real; unavailable variants do not silently fall back to props.

### Placement contract

The peer request carries a bounded spawn recipe, never a trusted position. At action execution the authority requires cheats, an Alive requesting player and loaded collision, then derives the eye from authoritative origin plus view height and traces along authoritative view angles to a fixed maximum range. The initial recipes require static map support: no miss, no start/all-solid result, and a walkable upward normal. The hit point is offset and settled using bounds supplied by the recipe, followed by a clearance trace. Weapons and bags use their small world bounds; an NPC uses the player hull. Dynamic players and entities do not become accidental spawn platforms in the first slice.

Placement returns either one finite origin/normal/yaw or a typed refusal such as `NotAllowed`, `NotAlive`, `NoWorld`, `NoSurface`, `SurfaceTooSteep`, `Blocked`, `UnsupportedModel`, `CapacityExhausted`, `ContentsDoNotFit`, or `RosterFull`. Recipe-specific creation is atomic after placement: a failed entity allocation, container fill, model-policy check or bot-slot allocation leaves no partial object. Successful and rejected requests produce reliable, request-correlated feedback; success reports the authoritative kind and final position.

Weapon, loot and NPC requests enter `TickInput` and execute inside `sim::step`, so authority, demo and replay see the same recipe and world state. Name and attachment resolution happens against installed session content before encoding bounded IDs; an NPC recipe carries one bounded model key that authority validates against its installed retained-model library. A loot-table roll consumes a dedicated deterministic match-RNG domain and table content participates in the session digest. Native NPC state is public fixed-size snapshot state with no client metadata, reliable recipient queue, player inventory or `UserCmd`. A non-host `spawn npc` is explicitly refused until remote host administration exists.

### Typed spawn products

- **Weapon:** expose a narrow stationary-spawn operation in the existing dropped-weapon subsystem. It creates a normal `DroppedItem`/`ET_ITEM`, uses the configured starting-ammo rules, retains the existing 16-drop policy, world model, touch-for-ammo behavior, aimed Use pickup and weapon-swap rules. It is not an `inventory::ItemInstance` and does not create weapon durability, attachments-as-items or a bridge into the grid inventory.
- **Loot:** create a `LootWorldContainer` record that owns an aggregate `GridContainer`, collision-safe `ContainerId`, world entity reference, presentation definition and pose. Direct item and loot-table recipes both populate the grid with authority-allocated `ItemInstanceId`s before the record becomes visible. World-created container and instance ID domains are disjoint from player-created IDs, and their next serials are snapshot state; transfer never changes an instance ID.
- **NPC:** own a bounded `NpcActors` registry keyed by a disjoint dynamic presence ID. A target uses one map XModel, player-hull placement clearance, native health and a bounds collision volume that enters the normal bullet, melee, projectile and splash damage paths. Its snapshot row contains only presentation pose, model and health; it has no client slot, `PlayerState`, loadout, HUD, scoreboard, killcam, player scripts or remote-player skinning. Later sentry/grunt behavior adds low-frequency intent to this actor rather than recreating the player lifecycle.

### One Use target

World bags must not add a second independent Use handler beside weapon pickup. Refactor aimed selection into one deterministic `UseTarget` arbitration covering dropped weapons, retrievable projectiles and loot containers. Candidates share distance, view-cone and line-of-sight scoring, then use an explicit kind priority and entity-number tie break. One Use edge dispatches to at most one target; automatic touch pickup for ammo and already-owned weapons remains separate.

An aimed bag publishes a private use hint. Use grants that player access to its `ContainerId` and causes the inventory overlay to open; it never auto-transfers an item. The authority rechecks entity generation, Alive state, range and line of sight, revoking on failure, death, removal or teardown. Opening another bag replaces the previous grant. Closing the overlay sends a typed release so an empty, unobserved bag can be removed deterministically. Multiple players may hold access to the same bag; expected revisions serialize races without partial transfer.

The world-container package therefore begins by lifting cross-container mutation into an authority-owned container registry. Player backpacks may retain their deterministic IDs, but lookup, access checks and atomic source/destination commits cannot remain methods that assume both IDs are the caller's backpack. Private snapshot projection sends a player only their backpack, their current grant and the public pose/presentation of world bags; bag contents remain hidden without a grant.

### Delivery order and acceptance

1. **Spawn foundation and weapon proof:** add the `spawn` console parser/completion, bounded weapon recipe/action/verdict codecs, authority look trace and stationary dropped-weapon constructor. Prove miss/steep/blocked/capacity refusals, successful pickup, snapshot adoption and replay determinism before adding another recipe.
2. **World loot pickup:** add the authority container registry and disjoint world ID allocators, direct-item and named-table bag recipes, one Use target, access release/revocation, private revision sync, overlay opening and player↔bag transfer. Empty bags disappear only after their last grant is released or revoked.
3. **Lightweight NPC foundation:** add `spawn npc target <map-model>` with bounded model completion, player-hull placement, native health, simple model-bounds collision, damage/death removal and snapshot/replay restoration. Then add low-frequency sentry targeting/firing and waypoint/direct-chase grunt movement. Keep full bots as an optional separate tier.

Headless artifact probes cover bounded codecs, atomic failure, deterministic placement and replay. Live acceptance covers visible weapon/bag presentation, pickup hints, one-press/one-target behavior, two-client privacy and transfer races. NPC acceptance proves that a target renders, takes ordinary weapon damage and disappears on death without adding a client, then measures 0/16/32 visible and offscreen actors before behavior work begins. No ordinary test or permanent probe is added without owner approval.

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

Dropped items are single stacks, not bags. A `WorldItem` holds one `ItemInstance` and owns a script-mover presence in the `0x7000_0000` range, drawn with the item's `world_model` through the existing spawned script-model path. The registry (`sim/src/inventory/world_items.rs`) is capped at `MAX_WORLD_ITEMS` and travels in `SnapshotMeta::world_items`. Dragging a backpack item off the grid sends `InventoryTransaction::Drop`, which places the stack on the floor in front of the player. Drops are refused when the item has no world model or the map does not load it. Aiming at a stack shows a plain-text Use hint, and Use moves the stack into the backpack, topping up matching stacks first and keeping the instance ID. Aggregate bags remain the plan for multi-item containers.

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

Status: complete on 2026-10-04.

Owned paths: new `crates/inventory_lab/` and `xtask/src/loot.rs`, plus minimal workspace/xtask registration. Depends only on catalog/grid core. Deliver `cargo xtask loot validate` and a maintained placeholder preview that exercises the real grid/UI components once they exist. Before UI exists, validate and print a compact catalog/layout report. Complete when a clean checkout can inspect all authored items without `IW4L_GAMES`.

### Simulation integration

Status: complete on 2026-10-04. Cross-container commits activate when an accessible world container exists.

Owned paths: new `crates/sim/src/inventory/` plus narrow hooks in input, step, world/frame, adoption, and snapshot metadata. Depends on catalog/grid core. Deliver player containers, deterministic instance allocation, revisioned transactions, typed rejection/accept events, canonical snapshots, teardown, and replay-safe cloning. No world entities or persistence. Complete when an artifact probe demonstrates accepted, overlapping, out-of-bounds, and stale-revision moves through `sim::step` and snapshot adoption.

### Network transport

Status: complete on 2026-10-04.

Owned paths: new `crates/net/src/transport/inventory_wire.rs` plus narrow registrations in action/meta/frame codecs and seat filtering. Depends on simulation integration. Deliver bounded action codecs, revision sync, private projection, decoder reconstruction, and authoritative hash coverage. Complete when wire round trips and malformed-input evidence are recorded and existing protocol callers build.

### Inventory overlay

Status: complete on 2026-10-04. In a match, Tab toggles the overlay and replaces the retail `+scores` bind on that key; the console keeps Tab for completion.

Owned paths: new `crates/inventory_ui/`, the generic modal contract in `frame`, and narrow plugin/input registration. Depends on catalog/grid core and presented simulation state; it may begin against the lab adapter before network transport is finished. Deliver two grids, drag/rotate, tooltip, pending UX, cursor/input capture, teardown, and placeholder icons. Complete when the same UI works in the asset-free lab and emits one transaction per completed drop.

### World spawn foundation and weapon pickup

Status: in progress. The typed `spawn weapon` path, authority placement, normal dropped-weapon creation and wire/replay semantics are complete; data-backed visual and pickup acceptance remains.

Owned paths: typed debug-spawn input/events and codecs, a console `spawn` dispatcher, the shared authority placement resolver, and a narrow stationary constructor in the existing weapon-item subsystem. Depends on simulation and transport. Deliver `spawn weapon`, completion from the installed weapon catalog, authority-derived look placement, reliable typed feedback and normal weapon pickup behavior. Complete when headless evidence proves deterministic placement/refusal and replay, and a data-backed run proves a spawned weapon can be seen and picked up. This package does not make weapons grid-inventory items.

### World loot containers

Owned paths: an authority-owned container registry and world ID allocators, a focused sim world-loot module, unified use-target integration, and `render_anim/src/occupancy/loot.rs`; session installs presentation facts. Depends on the world spawn foundation, simulation and transport. Deliver direct-item and named-table bag recipes, one aggregate bag model, access grant/release/revoke, use hint, overlay opening and player↔bag transfers. Complete when a two-client run proves distance/access checks, revision races and private contents, with rendering evidence deferred if game data is unavailable.

### Lightweight NPC actors

Status: foundation complete asset-free. `spawn npc target <map-model>` creates a native stationary actor with 100 health, simple bounds collision and snapshot/replay state; ordinary bullets, melee, direct projectiles and splash damage can remove it. Data-backed presentation and performance acceptance remain.

Owned paths: the native sim actor registry, debug-spawn recipe, bounded snapshot/wire state, ordinary damage routing, simple script-model presentation and the existing spawn console façade. Depends on the world spawn placement resolver and map XModel catalog. Next deliver a low-frequency stationary sentry, then waypoint/direct-chase grunt movement. Complete when live traces show materially lower authority, snapshot and render cost than client-backed dummies at 16 and 32 actors.

### Basic item effects

Owned paths: typed effect application inside sim and corresponding reliable UI feedback. Depends on the authoritative inventory. Deliver bandage/medkit healing and held-ammo restoration with lifecycle, health/ammo cap, consumption, interruption, and duplicate-request rules. Complete when replaying the same action stream produces the same inventory and player state.

### Persistence and extraction

Deferred until the match-scoped slice is stable. Do not store custom loot in the retail structured-data buffer. Design a schema-stamped loot account snapshot with atomic save and conflict detection, then extend account admission/ack transport deliberately. Stash and extraction rules are a separate owner decision and work package.

## Verification policy

This repository does not keep ordinary unit/probe tests outside owner-approved scenarios. Permanent catalog validation and invariant checking belong in the maintained authoring command. Disposable probes and their outputs live in the loot artifact. Add an `approved_tests` scenario only after the owner approves it by name.

Every package runs the narrowest relevant Cargo checks and records what could not be exercised without game data. Integration packages also check replay/wire determinism and `make publish-check`. Do not claim visual or map behavior from an asset-free preview.

## Current state and next handoff

- Implemented: the catalog/grid core and asset-free authoring tools; simulation-owned 8×6 player backpacks with stable item identities, deterministic authority-side transactions and private synchronization; the reusable `inventory_ui`; `loot_grant`; single-stack world-item drop/Use pickup; and `spawn weapon`. `spawn npc target <map-model>` now completes from retained map models and creates a bounded native `NpcActor` at the authority-derived floor point. The actor owns 100 health, a simple bounds hit volume, a dynamic script-model presentation owner and snapshot/replay state, while deliberately owning no client slot, `PlayerState`, reliable channel, inventory, HUD or bot controller. The game protocol is version 108.
- Verified: `cargo check --workspace --all-targets`; permanent catalog/lab checks; the earlier inventory and weapon probes; a new action/snapshot wire round trip; an asset-free authority probe that spawned entity 26 at `59.9,0.0,2.1`, reproduced actor state under replay, hit its model-bounds collider and killed/removed it with an ordinary weapon shot; and `make publish-check`. Warnings-denied Clippy remains blocked by extensive pre-existing `sim`/`net`/`console` lint debt; the touched-file filter reported no new NPC/console/codec warning.
- Not verified: a lightweight target rendered from a real map model; comparative 0/16/32 actor performance; actor animation, intent, attacks, movement, scoring, death effects or loot drops; splash line of sight; a spawned weapon's live model/pickup; the inventory overlay in a retail-data match; multiplayer bandwidth; or persistence.
- Current blocker: live map/render acceptance needs legally obtained game data. Catalog, grid, authoring tools, simulation, codecs, and placeholder UI are not blocked.
- Single-stack world items: `base:soda` (`food_soda_single01`) and `base:chips` (`food_snacks_chips01`), drag-off-grid and `loot_drop <item>` drops, Use pickup with a full-backpack hint, introduced in protocol 107. Quick match runs on `mp_rundown`, which loads both models. Checked with `cargo check --workspace --all-targets`, `cargo xtask loot validate` and no-deps Clippy; not yet played in a live match.
- Next loot package: give the four existing items world models from the prop table, then implement **Basic item effects** so eating and drinking work. The weapon-spawn visual acceptance and aggregate bags remain open behind those.
- Next NPC package: add low-frequency **lightweight sentry** perception and firing atop `NpcActor`, retaining fixed-size public state and no client lifecycle. When game data is available, visually accept the target and record 0/16/32 visible/offscreen performance before mobile grunt navigation. Do not begin persistence or turn weapons into grid-inventory instances.
- Owner decisions still open: inventory/death retention policy; default backpack dimensions; missing runtime presentation policy; whether rifle ammo targets the held weapon or an authored weapon family.

## Working notes

Facts that cost time to rediscover. Keep this section current.

### World models and maps

- A world model renders only if the current map loads it. The sim model library is every xmodel in the map zone plus the `common_mp` models that scripts reference. Most props are map-specific.
- `context/artifacts/2026-10-04-loot-inventory/ff_model_names.py <zone.ff> [--models]` lists model names in a retail zone. It dechunks signed zones the same way as `asset_transport::zone::dechunk_authed`.
- Prop candidates found so far:

| Model | Use | Maps |
|---|---|---|
| `food_soda_single01`, `food_soda_single02` | soda can (`base:soda`) | boneyard, derail, favela, highrise, invasion, nightshift, quarry, rundown, subbase, terminal, underpass |
| `food_snacks_chips01` | chips (`base:chips`) | most MP maps except rust |
| `food_snacks_beefjerky01`, `_cookies01`, `_donuts01`, `_peanuts01`, `_krustbar01` | snacks | most MP maps except rust |
| `food_soda_sixpack01` | six-pack | estate, favela, rundown, underpass |
| `com_cellphone` | phone | most MP maps |
| `com_red_toolbox` | tools | checkpoint, highrise, nightshift, quarry, rundown, subbase |
| `com_plasticcase_green_rifle` | ammo/weapon case | rundown (others unchecked) |

- `mp_rust` loads almost no small props. Quick match runs on `mp_rundown` for that reason.

### Sim and wire

- World items render through the spawned script-model path. A sim script mover plus a collision owner with a dobj is enough; the renderer draws presences in `0x4000_0000..0x8000_0000`. World items use `0x7000_0000` and up, and GSC spawns stop below that.
- `authoritative_snapshot_hash` hashes the encoded wire frame. A new `SnapshotMeta` field needs a codec in `meta_wire.rs` and a `PROTOCOL_VERSION` bump, or hashing and peer adoption miss it.
- Snapshot adoption restores movers but not collision owners. `world_items::restore_owners` rebuilds them; any new world object needs the same.
- Level restart clears world items in `script/host/restart.rs`.
- `ItemDefId` follows sorted key order, so adding an item renumbers others. Never persist raw definition IDs.
- Pickup reuses the dropped-weapon Use selection in `item.rs`. Weapons, projectiles and world items share one score, so the nearest well-aimed target wins.
- Hints use plain text: `HUD_STRING_PLAIN` prefix, `&&1` for the Use key, `cursor_hint = 1`.

### Testing

The owner runs live tests. Agents stop at `cargo check --workspace --all-targets`, `cargo xtask loot validate` and no-deps Clippy (`cargo clippy -p inventory@0.1.0 -p inventory_ui --no-deps -- -D warnings`), then hand over steps. A typical live check in quick match:

```text
loot_grant base:soda 3
loot_drop base:soda        # or Tab and drag off the grid
# look at the can: hint shows; Use picks it up with a toast
```

## Session finish

Before handing off, update this section with implemented facts, verification, gaps, and the exact next package. Add a new artifact iteration with evidence and a mandatory “what was not done” section. State whether scaffolding was removed, retained as a maintained tool, or never added. Leave prior artifact iterations unchanged.
