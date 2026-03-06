use anyhow::{Context, bail};
use assets::{NodeGraphFile, SceneFile, validate_node_graph};
use bevy_ecs::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::str::FromStr;

#[derive(Component, Debug, Clone)]
pub struct EntityName {
    pub name: String,
}

#[derive(Component, Debug, Clone, Copy)]
pub struct Transform {
    pub translation: [f32; 3],
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            translation: [0.0, 0.0, 0.0],
        }
    }
}

#[derive(Component, Debug, Clone)]
pub struct MeshHandle {
    pub name: String,
}

#[derive(Component, Debug, Clone, Default)]
pub struct DynamicComponents {
    pub values: BTreeMap<String, Value>,
}

#[derive(Resource, Debug, Clone)]
pub struct SceneInfo {
    pub name: String,
}

pub struct SceneWorld {
    world: World,
}

impl SceneWorld {
    pub fn from_scene(scene: &SceneFile) -> Self {
        let mut world = World::new();
        populate_world_from_scene(&mut world, scene);
        Self { world }
    }

    pub fn rebuild_from_scene(&mut self, scene: &SceneFile) {
        self.world = World::new();
        populate_world_from_scene(&mut self.world, scene);
    }

    pub fn entity_count(&self) -> usize {
        self.world.entities().len() as usize
    }

    pub fn scene_name(&self) -> Option<String> {
        self.world
            .get_resource::<SceneInfo>()
            .map(|info| info.name.clone())
    }

    pub fn has_entity(&self, name: &str) -> bool {
        self.find_entity(name).is_some()
    }

    pub fn get_transform(&self, entity_name: &str) -> Option<[f32; 3]> {
        let entity = self.find_entity(entity_name)?;
        self.world
            .get::<Transform>(entity)
            .map(|transform| transform.translation)
    }

    pub fn set_transform(
        &mut self,
        entity_name: &str,
        translation: [f32; 3],
    ) -> anyhow::Result<()> {
        let entity = self
            .find_entity(entity_name)
            .with_context(|| format!("entity '{}' not found in ECS world", entity_name))?;
        let mut entity_ref = self.world.entity_mut(entity);
        let mut transform = entity_ref
            .get_mut::<Transform>()
            .with_context(|| format!("entity '{}' missing Transform component", entity_name))?;
        transform.translation = translation;
        Ok(())
    }

    pub fn upsert_dynamic_component(
        &mut self,
        entity_name: &str,
        component_type: &str,
        data: Value,
    ) -> anyhow::Result<()> {
        if component_type.trim().is_empty() {
            bail!("component_type cannot be empty");
        }
        let entity = self
            .find_entity(entity_name)
            .with_context(|| format!("entity '{}' not found in ECS world", entity_name))?;
        let mut entity_ref = self.world.entity_mut(entity);
        if let Some(mut dyn_components) = entity_ref.get_mut::<DynamicComponents>() {
            dyn_components
                .values
                .insert(component_type.to_string(), data.clone());
        } else {
            let mut values = BTreeMap::new();
            values.insert(component_type.to_string(), data.clone());
            entity_ref.insert(DynamicComponents { values });
        }
        Ok(())
    }

    pub fn get_dynamic_components(&self, entity_name: &str) -> Option<BTreeMap<String, Value>> {
        let entity = self.find_entity(entity_name)?;
        self.world
            .get::<DynamicComponents>(entity)
            .map(|components| components.values.clone())
    }

    fn find_entity(&self, entity_name: &str) -> Option<Entity> {
        self.world.iter_entities().find_map(|entity_ref| {
            let name = entity_ref.get::<EntityName>()?;
            if name.name == entity_name {
                Some(entity_ref.id())
            } else {
                None
            }
        })
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum GraphEvent {
    OnStart,
    OnUpdate,
    OnTriggerEnter,
}

impl GraphEvent {
    pub fn parse(value: &str) -> Option<Self> {
        Self::from_str(value).ok()
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::OnStart => "OnStart",
            Self::OnUpdate => "OnUpdate",
            Self::OnTriggerEnter => "OnTriggerEnter",
        }
    }
}

impl FromStr for GraphEvent {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "onstart" => Ok(Self::OnStart),
            "onupdate" => Ok(Self::OnUpdate),
            "ontriggerenter" => Ok(Self::OnTriggerEnter),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GraphSideEffect {
    SpawnEntity {
        entity_name: String,
        mesh: String,
        translation: [f32; 3],
    },
    MoveEntity {
        entity_name: String,
        translation: [f32; 3],
    },
    ApplyDamage {
        entity_name: String,
        amount: f32,
    },
    SetLightPreset {
        preset: String,
    },
    SetWeather {
        preset: String,
    },
    ShowMessage {
        text: String,
    },
    SetObjective {
        text: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GraphExecutionLog {
    pub phase: String,
    pub node_id: String,
    pub node_type: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct GraphExecutionSummary {
    pub triggered_events: Vec<String>,
    pub executed_node_ids: Vec<String>,
    pub side_effects: Vec<GraphSideEffect>,
    pub logs: Vec<GraphExecutionLog>,
}

pub fn execute_runtime_graph(
    graph: &NodeGraphFile,
    events: &[GraphEvent],
) -> anyhow::Result<GraphExecutionSummary> {
    let report = validate_node_graph(graph);
    if !report.valid {
        bail!("graph validation failed: {}", report.errors.join("; "));
    }

    let mut node_by_id = HashMap::<String, (String, Value)>::new();
    for node in &graph.nodes {
        node_by_id.insert(
            node.id.clone(),
            (node.node_type.clone(), node.params.clone()),
        );
    }

    let mut adjacency = HashMap::<String, Vec<String>>::new();
    let mut indegree = HashMap::<String, usize>::new();
    for node in &graph.nodes {
        indegree.insert(node.id.clone(), 0);
    }
    for edge in &graph.edges {
        adjacency
            .entry(edge.from.clone())
            .or_default()
            .push(edge.to.clone());
        if let Some(entry) = indegree.get_mut(&edge.to) {
            *entry += 1;
        }
    }
    for children in adjacency.values_mut() {
        children.sort();
        children.dedup();
    }

    let mut topo_queue = BTreeSet::<String>::new();
    let mut indegree_mut = indegree.clone();
    for (node_id, degree) in &indegree_mut {
        if *degree == 0 {
            topo_queue.insert(node_id.clone());
        }
    }

    let mut topo = Vec::<String>::new();
    while let Some(next) = topo_queue.iter().next().cloned() {
        topo_queue.remove(&next);
        topo.push(next.clone());
        if let Some(children) = adjacency.get(&next) {
            for child in children {
                if let Some(entry) = indegree_mut.get_mut(child) {
                    *entry = entry.saturating_sub(1);
                    if *entry == 0 {
                        topo_queue.insert(child.clone());
                    }
                }
            }
        }
    }
    if topo.len() != graph.nodes.len() {
        bail!("graph execution requires a DAG; cycle detected");
    }

    let events_set = events.iter().copied().collect::<HashSet<GraphEvent>>();
    let mut roots = graph
        .nodes
        .iter()
        .filter_map(|node| {
            let node_event = GraphEvent::parse(node.node_type.as_str())?;
            if events_set.contains(&node_event) {
                Some(node.id.clone())
            } else {
                None
            }
        })
        .collect::<Vec<String>>();
    roots.sort();

    let mut reachable = HashSet::<String>::new();
    let mut stack = roots.clone();
    while let Some(node_id) = stack.pop() {
        if !reachable.insert(node_id.clone()) {
            continue;
        }
        if let Some(children) = adjacency.get(&node_id) {
            for child in children.iter().rev() {
                stack.push(child.clone());
            }
        }
    }

    let mut summary = GraphExecutionSummary {
        triggered_events: events
            .iter()
            .map(|event| event.as_str().to_string())
            .collect(),
        ..GraphExecutionSummary::default()
    };
    for root in &roots {
        if let Some((node_type, _)) = node_by_id.get(root) {
            summary.logs.push(GraphExecutionLog {
                phase: "event_collection".to_string(),
                node_id: root.clone(),
                node_type: node_type.clone(),
                message: "event root collected".to_string(),
            });
        }
    }

    for node_id in topo {
        if !reachable.contains(&node_id) {
            continue;
        }
        let Some((node_type, params)) = node_by_id.get(&node_id) else {
            continue;
        };
        summary.executed_node_ids.push(node_id.clone());
        summary.logs.push(GraphExecutionLog {
            phase: "flow_execution".to_string(),
            node_id: node_id.clone(),
            node_type: node_type.clone(),
            message: "node executed".to_string(),
        });
        let effects = node_side_effects(node_id.as_str(), node_type.as_str(), params);
        for effect in effects {
            summary.logs.push(GraphExecutionLog {
                phase: "side_effect_commit".to_string(),
                node_id: node_id.clone(),
                node_type: node_type.clone(),
                message: "side effect committed".to_string(),
            });
            summary.side_effects.push(effect);
        }
    }

    Ok(summary)
}

fn node_side_effects(node_id: &str, node_type: &str, params: &Value) -> Vec<GraphSideEffect> {
    match node_type {
        "SpawnEntity" => {
            let count = params
                .get("count")
                .and_then(Value::as_u64)
                .map(|value| value as usize)
                .unwrap_or(1)
                .clamp(1, 64);
            let mesh = params
                .get("mesh")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("cube")
                .to_string();
            let base_name = params
                .get("base_name")
                .or_else(|| params.get("prefab"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("SpawnedEntity");
            let origin = params
                .get("origin")
                .and_then(Value::as_array)
                .filter(|array| array.len() == 3)
                .map(|array| {
                    [
                        array[0].as_f64().unwrap_or(0.0) as f32,
                        array[1].as_f64().unwrap_or(0.0) as f32,
                        array[2].as_f64().unwrap_or(0.0) as f32,
                    ]
                })
                .unwrap_or([0.0, 0.0, 0.0]);
            (0..count)
                .map(|index| GraphSideEffect::SpawnEntity {
                    entity_name: if count == 1 {
                        format!("{}_{}", base_name, node_id)
                    } else {
                        format!("{}_{}_{}", base_name, node_id, index + 1)
                    },
                    mesh: mesh.clone(),
                    translation: [origin[0] + index as f32 * 1.5, origin[1], origin[2]],
                })
                .collect()
        }
        "MoveTo" => {
            let entity_name = params
                .get("entity")
                .or_else(|| params.get("entity_name"))
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or("");
            let position = params
                .get("position")
                .and_then(Value::as_array)
                .filter(|array| array.len() == 3)
                .map(|array| {
                    [
                        array[0].as_f64().unwrap_or(0.0) as f32,
                        array[1].as_f64().unwrap_or(0.0) as f32,
                        array[2].as_f64().unwrap_or(0.0) as f32,
                    ]
                })
                .unwrap_or([0.0, 0.0, 0.0]);
            if entity_name.is_empty() {
                Vec::new()
            } else {
                vec![GraphSideEffect::MoveEntity {
                    entity_name: entity_name.to_string(),
                    translation: position,
                }]
            }
        }
        "ApplyDamage" => {
            let entity_name = params
                .get("target")
                .or_else(|| params.get("entity"))
                .or_else(|| params.get("entity_name"))
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or("");
            let amount = params.get("amount").and_then(Value::as_f64).unwrap_or(10.0) as f32;
            if entity_name.is_empty() {
                Vec::new()
            } else {
                vec![GraphSideEffect::ApplyDamage {
                    entity_name: entity_name.to_string(),
                    amount: amount.max(0.0),
                }]
            }
        }
        "SetLightPreset" => params
            .get("preset")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|preset| !preset.is_empty())
            .map(|preset| {
                vec![GraphSideEffect::SetLightPreset {
                    preset: preset.to_string(),
                }]
            })
            .unwrap_or_default(),
        "SetWeather" => params
            .get("preset")
            .or_else(|| params.get("weather"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|preset| !preset.is_empty())
            .map(|preset| {
                vec![GraphSideEffect::SetWeather {
                    preset: preset.to_string(),
                }]
            })
            .unwrap_or_default(),
        "ShowMessage" => params
            .get("text")
            .or_else(|| params.get("message"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(|text| {
                vec![GraphSideEffect::ShowMessage {
                    text: text.to_string(),
                }]
            })
            .unwrap_or_default(),
        "SetObjective" => params
            .get("text")
            .or_else(|| params.get("objective"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(|text| {
                vec![GraphSideEffect::SetObjective {
                    text: text.to_string(),
                }]
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn populate_world_from_scene(world: &mut World, scene: &SceneFile) {
    world.insert_resource(SceneInfo {
        name: scene.name.clone(),
    });

    for entity in &scene.entities {
        world.spawn((
            EntityName {
                name: entity.name.clone(),
            },
            Transform {
                translation: entity.translation,
            },
            MeshHandle {
                name: entity.mesh.clone(),
            },
            DynamicComponents::default(),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assets::{GRAPH_SCHEMA_VERSION, NodeGraphEdge, NodeGraphNode};
    use serde_json::json;

    #[test]
    fn execute_runtime_graph_is_deterministic() {
        let graph = NodeGraphFile {
            version: GRAPH_SCHEMA_VERSION,
            graph_name: "deterministic".to_string(),
            nodes: vec![
                NodeGraphNode {
                    id: "start".to_string(),
                    node_type: "OnStart".to_string(),
                    params: json!({}),
                },
                NodeGraphNode {
                    id: "spawn".to_string(),
                    node_type: "SpawnEntity".to_string(),
                    params: json!({
                        "base_name": "Enemy",
                        "mesh": "cube",
                        "origin": [0.0,0.0,0.0],
                        "count": 2
                    }),
                },
                NodeGraphNode {
                    id: "objective".to_string(),
                    node_type: "SetObjective".to_string(),
                    params: json!({ "text": "Clear wave" }),
                },
            ],
            edges: vec![
                NodeGraphEdge {
                    from: "start".to_string(),
                    to: "spawn".to_string(),
                    pin: "flow".to_string(),
                },
                NodeGraphEdge {
                    from: "spawn".to_string(),
                    to: "objective".to_string(),
                    pin: "flow".to_string(),
                },
            ],
        };
        let first = execute_runtime_graph(&graph, &[GraphEvent::OnStart])
            .expect("graph execution should succeed");
        let second = execute_runtime_graph(&graph, &[GraphEvent::OnStart])
            .expect("graph execution should succeed");
        assert_eq!(first.executed_node_ids, second.executed_node_ids);
        assert_eq!(first.side_effects, second.side_effects);
        assert_eq!(first.executed_node_ids, vec!["start", "spawn", "objective"]);
    }

    #[test]
    fn execute_runtime_graph_requires_valid_graph() {
        let graph = NodeGraphFile {
            version: GRAPH_SCHEMA_VERSION,
            graph_name: "invalid".to_string(),
            nodes: vec![NodeGraphNode {
                id: "a".to_string(),
                node_type: "OnStart".to_string(),
                params: json!({}),
            }],
            edges: vec![NodeGraphEdge {
                from: "a".to_string(),
                to: "missing".to_string(),
                pin: "flow".to_string(),
            }],
        };
        let result = execute_runtime_graph(&graph, &[GraphEvent::OnStart]);
        assert!(result.is_err());
    }
}
