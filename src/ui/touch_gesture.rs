use std::collections::HashMap;

use egui::{Context, Id, Pos2, Rect, Vec2};

/// Two-finger pan/pinch over `region`.
///
/// egui has no multi-touch gesture recognizer of its own: `ScrollArea` only
/// drag-scrolls with one pointer and `Event::Magnify` is emitted for macOS
/// trackpads, not touch screens. So views that want to navigate with two
/// fingers call this and apply the result to their own scroll/zoom state.
///
/// Returns `(centroid_delta, zoom_scale, centroid)` while at least two fingers
/// are inside `region`, or `None` otherwise. `zoom_scale` is `1.0` when the
/// fingers are not pinching. `id_salt` namespaces the stored gesture state and
/// must differ between regions that can be pinched at once.
pub fn two_finger(ctx: &Context, region: Rect, id_salt: &str) -> Option<(Vec2, f32, Pos2)> {
    let id_centroid = Id::new((id_salt, "centroid"));
    let id_dist = Id::new((id_salt, "dist"));

    //NOTE: On Android a single frame may contain multiple Event::Touch entries
    let mut seen: HashMap<u64, Pos2> = ctx.input(|i| {
        let mut map = HashMap::new();
        for e in &i.events {
            if let egui::Event::Touch { id, pos, phase, .. } = e {
                matches!(phase, egui::TouchPhase::Start | egui::TouchPhase::Move).then(|| {
                    if region.contains(*pos) {
                        map.insert(id.0, *pos);
                    }
                });
            }
        }
        map
    });

    // Two-finger pan/pinch requires at least two distinct touch points.
    if seen.len() < 2 {
        ctx.memory_mut(|m| {
            m.data.remove::<Pos2>(id_centroid);
            m.data.remove::<f32>(id_dist);
        });
        return None;
    }

    // Pop two arbitrary points for the centroid / distance calculation.
    let p1 = seen.values().next().copied().unwrap();
    seen.remove(&seen.keys().next().copied().unwrap());
    let p2 = seen.values().next().copied().unwrap();

    let centroid = Pos2::new((p1.x + p2.x) * 0.5, (p1.y + p2.y) * 0.5);
    let dist = (p1 - p2).length();

    let prev = ctx.memory(|m| {
        (
            m.data.get_temp::<Pos2>(id_centroid),
            m.data.get_temp::<f32>(id_dist),
        )
    });

    ctx.memory_mut(|m| {
        m.data.insert_temp(id_centroid, centroid);
        m.data.insert_temp(id_dist, dist);
    });

    let prev_centroid = prev.0?;
    let scale = match prev.1 {
        Some(prev_dist) if prev_dist > 1.0 => (dist / prev_dist).clamp(0.5, 2.0),
        _ => 1.0,
    };

    Some((centroid - prev_centroid, scale, centroid))
}
