# Low-code MVP Spec (Phase 4)

## Node types in MVP

- Event nodes: `OnStart`, `OnUpdate`, `OnTriggerEnter`
- Flow nodes: `Sequence`, `Branch`, `Delay`
- Gameplay nodes: `SpawnEntity`, `MoveTo`, `PlayAnimation`, `ApplyDamage`
- Scene nodes: `SetLightPreset`, `LoadSubScene`, `SetWeather`
- UI nodes: `ShowMessage`, `SetObjective`
- Utility nodes: `GetEntity`, `GetPlayer`, `RandomFloat`

## JSON schema (runtime graph)

```json
{
  "version": 1,
  "graph_name": "template_shooter",
  "nodes": [
    {
      "id": "n1",
      "type": "OnStart",
      "params": {}
    },
    {
      "id": "n2",
      "type": "SpawnEntity",
      "params": { "prefab": "enemy_grunt", "count": 3 }
    }
  ],
  "edges": [
    { "from": "n1", "to": "n2", "pin": "flow" }
  ]
}
```

## Runtime execution model

- Parse graph JSON to IR at load
- Validate node contract and pin compatibility
- Build topological order per event root
- Execute in deterministic phases:
  - Phase A: event collection
  - Phase B: flow execution
  - Phase C: side-effect commit (spawn/destroy/state writes)
- Emit per-node execution logs and timing

## 1-click templates

- `template_shooter_arena`
  - Player spawn, enemy waves, score objective, ammo pickups

- `template_medieval_island`
  - Terrain, village cluster, quest NPCs, day/night cycle preset

- `template_platform_runner`
  - Start/goal, checkpoint chain, moving platforms, hazard triggers

## Done criteria

- User creates a playable prototype by selecting one template
- User edits at least 3 nodes without code
- Runtime executes graph with traceable logs