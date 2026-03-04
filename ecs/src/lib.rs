use assets::SceneFile;
use bevy_ecs::prelude::*;

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
        world.insert_resource(SceneInfo {
            name: scene.name.clone(),
        });

        for entity in &scene.entities {
            world.spawn((
                Transform {
                    translation: entity.translation,
                },
                MeshHandle {
                    name: entity.mesh.clone(),
                },
            ));
        }

        Self { world }
    }

    pub fn entity_count(&self) -> usize {
        self.world.entities().len() as usize
    }
}
