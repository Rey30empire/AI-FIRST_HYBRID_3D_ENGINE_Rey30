# Vertical Slice WOW Definition (Phase 2)

Target: one interior and one exterior scene that immediately signal "premium visual quality" while staying interactive.

## Visual stack (must-have)

- PBR materials (metallic/roughness)
- HDR pipeline with tone mapping
- Real-time shadows (directional + spot as needed)
- Bloom
- Light volumetric/height fog
- Color grading presets

## Required content set

- Interior: enclosed environment with mixed direct/indirect lighting and glossy surfaces
- Exterior: open terrain/architecture with sky lighting and long-range visibility

## Cinematic presets

- Natural Day
- Golden Hour
- Noir Indoor

Each preset controls: exposure, white balance, bloom intensity, fog density, LUT.

## Done criteria

- User can switch interior/exterior in editor
- User can switch presets in one click
- Frame pacing remains stable at target hardware tier
- Visual capture pack exported (before/after presets)