use assets::{SceneEntity, SceneFile};

pub fn build_scene_from_prompt(prompt: &str) -> SceneFile {
    let lower = prompt.to_ascii_lowercase();
    if lower.contains("shooter") {
        return shooter_scene();
    }
    if lower.contains("plataforma") || lower.contains("platform") || lower.contains("runner") {
        return platform_scene();
    }
    if lower.contains("island") || lower.contains("medieval") || lower.contains("isla") {
        return island_scene();
    }
    generic_scene(prompt)
}

fn shooter_scene() -> SceneFile {
    SceneFile {
        name: "Generated Shooter Arena".to_string(),
        entities: vec![
            SceneEntity {
                name: "PlayerSpawn".to_string(),
                mesh: "cube".to_string(),
                translation: [0.0, 0.0, 0.0],
            },
            SceneEntity {
                name: "EnemySpawn_A".to_string(),
                mesh: "cube".to_string(),
                translation: [4.0, 0.0, -3.0],
            },
            SceneEntity {
                name: "EnemySpawn_B".to_string(),
                mesh: "cube".to_string(),
                translation: [-4.0, 0.0, -3.0],
            },
        ],
    }
}

fn island_scene() -> SceneFile {
    SceneFile {
        name: "Generated Medieval Island".to_string(),
        entities: vec![
            SceneEntity {
                name: "IslandTerrain".to_string(),
                mesh: "cube".to_string(),
                translation: [0.0, -1.0, 0.0],
            },
            SceneEntity {
                name: "CastleCore".to_string(),
                mesh: "cube".to_string(),
                translation: [2.0, 0.5, -1.0],
            },
            SceneEntity {
                name: "VillageCluster".to_string(),
                mesh: "cube".to_string(),
                translation: [-2.5, 0.2, 1.5],
            },
        ],
    }
}

fn platform_scene() -> SceneFile {
    SceneFile {
        name: "Generated Platform Runner".to_string(),
        entities: vec![
            SceneEntity {
                name: "StartPlatform".to_string(),
                mesh: "cube".to_string(),
                translation: [0.0, 0.0, 0.0],
            },
            SceneEntity {
                name: "JumpPlatform_A".to_string(),
                mesh: "cube".to_string(),
                translation: [2.0, 0.5, -1.0],
            },
            SceneEntity {
                name: "GoalPlatform".to_string(),
                mesh: "cube".to_string(),
                translation: [5.0, 1.0, -2.0],
            },
        ],
    }
}

fn generic_scene(prompt: &str) -> SceneFile {
    SceneFile {
        name: format!("Generated Scene: {}", prompt.trim()),
        entities: vec![SceneEntity {
            name: "RootProp".to_string(),
            mesh: "cube".to_string(),
            translation: [0.0, 0.0, 0.0],
        }],
    }
}
