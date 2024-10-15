use cs2::{EntitySystem, CEntityIdentityEx};
use nalgebra::Vector3;
use obfstr::obfstr;
use cs2::BoneFlags;
use cs2::CS2Model;
use cs2::ClassNameCache;
use cs2::PlayerPawnState;
use crate::UnicodeTextRenderer;

use crate::settings::AppSettings;
use crate::view::{KeyToggle, ViewController};
use crate::UpdateContext;
use super::Enhancement;
use valthrun_kernel_interface::MouseState;

pub struct Aimbot {
    toggle: KeyToggle,
    fov: f32,
    aim_speed_x: f32,
    aim_speed_y: f32,
    is_active: bool,
    current_target: Option<[f32; 2]>,
    aim_bone: String,
    aimbot_team_check: bool,
}


impl Aimbot {
    pub fn new() -> Self {
        Self {
            toggle: KeyToggle::new(),
            fov: 3.0,
            aim_speed_x: 2.5,
            aim_speed_y: 2.5,
            is_active: false,
            current_target: None,
            aim_bone: "head".to_string(),
            aimbot_team_check: true,
        }
    }

    fn world_to_screen(&self, view: &ViewController, world_position: &Vector3<f32>) -> Option<[f32; 2]> {
        view.world_to_screen(world_position, true).map(|vec| [vec.x, vec.y])
    }

    fn find_best_target(&mut self, ctx: &UpdateContext) -> Option<[f32; 2]> {
        let settings = ctx.states.resolve::<AppSettings>(()).ok()?;
        let entities = ctx.states.resolve::<EntitySystem>(()).ok()?;
        let view = ctx.states.resolve::<ViewController>(()).ok()?;
        let class_name_cache = ctx.states.resolve::<ClassNameCache>(()).ok()?; 
    
        let local_player_position = view.get_camera_world_position()?;
        let local_player_controller = entities.get_local_player_controller().ok()?;
        let local_player_controller = local_player_controller.reference_schema().ok()?;
        let crosshair_pos = [view.screen_bounds.x / 2.0, view.screen_bounds.y / 2.0];
        let mut best_target: Option<[f32; 2]> = None;
        let mut lowest_distance_from_crosshair = f32::MAX;
        const UNITS_TO_METERS: f32 = 0.01905;
    
        for entity_identity in entities.all_identities() {
            let entity_class = class_name_cache.lookup(&entity_identity.entity_class_info().ok()?).ok()?;
            if entity_class.map(|name| *name == "C_CSPlayerPawn").unwrap_or(false) {
                let entry = ctx.states.resolve::<PlayerPawnState>(entity_identity.handle::<()>().ok()?.get_entity_index()).ok()?;
                if let PlayerPawnState::Alive(player_info) = &*entry {
                    let entry_model = ctx.states.resolve::<CS2Model>(player_info.model_address).ok()?;
    
                    if settings.aimbot_team_check && local_player_controller.m_iTeamNum().unwrap() == player_info.team_id {
                        continue;
                    }
    
                    let distance = (player_info.position - local_player_position).norm() * UNITS_TO_METERS;
                    if distance < 2.0 {
                        continue;
                    }
    
                    // Iterate through all bones and select the closest one
                    for (bone, state) in entry_model.bones.iter().zip(player_info.bone_states.iter()) {
                        if (bone.flags & BoneFlags::FlagHitbox as u32) == 0 {
                            continue;
                        }
    
                        // If "closest" is chosen, we don't filter by bone name
                        if settings.aim_bone == "closest" || bone.name.to_lowercase().contains(&settings.aim_bone) {
                            if let Some(screen_position) = self.world_to_screen(&view, &state.position) {
                                let dx = screen_position[0] - crosshair_pos[0];
                                let dy = screen_position[1] - crosshair_pos[1];
                                let distance_from_crosshair = (dx * dx + dy * dy).sqrt();
    
                                let angle_to_target = distance_from_crosshair.atan2(view.screen_bounds.x / 2.0).to_degrees();
    
                                if angle_to_target <= self.fov / 2.0 {
                                    if distance_from_crosshair < lowest_distance_from_crosshair {
                                        lowest_distance_from_crosshair = distance_from_crosshair;
                                        best_target = Some(screen_position);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    
        best_target
    }

    fn aim_at_target(&self, ctx: &UpdateContext, target_screen_position: [f32; 2]) -> anyhow::Result<bool> {
        let view = ctx.states.resolve::<ViewController>(())?;
        let crosshair_pos = [view.screen_bounds.x / 2.0, view.screen_bounds.y / 2.0];
    
        let aim_adjustment = [
            (target_screen_position[0] - crosshair_pos[0]) / self.aim_speed_x,
            (target_screen_position[1] - crosshair_pos[1]) / self.aim_speed_y,
        ];
    
        ctx.cs2.send_mouse_state(&[MouseState {
            last_x: aim_adjustment[0] as i32,
            last_y: aim_adjustment[1] as i32,
            ..Default::default()
        }])?;
    
        Ok(true)
    }
    
}

impl Enhancement for Aimbot {
    fn update(&mut self, ctx: &UpdateContext) -> anyhow::Result<()> {
        let settings = ctx.states.resolve::<AppSettings>(())?;
    
        self.fov = settings.aimbot_fov;
        self.aim_speed_x = settings.aimbot_speed_x;
        self.aim_speed_y = settings.aimbot_speed_y;
        self.aim_bone = settings.aim_bone.to_lowercase();
        self.aimbot_team_check = settings.aimbot_team_check;
    
        if self.toggle.update_dual(
            &settings.aimbot_mode,
            ctx.input,
            &settings.key_aimbot,
            &settings.key_aimbot_secondary,
        ) {
            ctx.cs2.add_metrics_record(
                obfstr!("feature-aimbot-toggle"),
                &format!("enabled: {}, mode: {:?}", self.toggle.enabled, settings.aimbot_mode),
            );
        }
    
        if self.toggle.enabled {
            if let Some(target_screen_position) = self.find_best_target(ctx) {
                self.aim_at_target(ctx, target_screen_position)?;
            } else {
                self.current_target = None;
            }
        }
    
        Ok(())
    }

    fn render(&self, _states: &utils_state::StateRegistry, _ui: &imgui::Ui, _unicode_text: &UnicodeTextRenderer) -> anyhow::Result<()> {
        Ok(())
    }
}
