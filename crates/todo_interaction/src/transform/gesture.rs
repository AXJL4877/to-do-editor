use super::align::{snap_move, snap_resize, AlignGuide, AlignSnap};
use super::hit_test::TransformHit;
use todo_core::{BaseSize, Transform, TransformBounds};

const MIN_SIZE_PX: f64 = 1.0;

#[derive(Debug, Clone)]
pub(crate) struct TransformGesture {
    original: Transform,
    // A capture keeps its starting geometry even if decoder metadata arrives.
    base: BaseSize,
    pointer: (f64, f64),
    hit: TransformHit,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GestureUpdate {
    pub transform: Transform,
    pub guides: Vec<AlignGuide>,
}

impl TransformGesture {
    pub(crate) fn new(
        original: Transform,
        base: BaseSize,
        pointer: (f64, f64),
        hit: TransformHit,
    ) -> Self {
        Self {
            original,
            base,
            pointer,
            hit,
        }
    }

    pub(crate) fn update(
        &self,
        pointer: (f64, f64),
        canvas: (f64, f64),
        peers: &[TransformBounds],
        snapping: bool,
    ) -> Option<GestureUpdate> {
        let bounds = self.original.bounds_for_base(self.base)?;
        let (dx, dy) = (pointer.0 - self.pointer.0, pointer.1 - self.pointer.1);
        let mut transform = self.original;
        let mut guides = Vec::new();
        match self.hit {
            TransformHit::Move => {
                transform.x += dx;
                transform.y += dy;
                let Some(moved) = transform.bounds_for_base(self.base) else {
                    return transform
                        .is_valid()
                        .then_some(GestureUpdate { transform, guides });
                };
                let AlignSnap {
                    delta,
                    guides: snap_guides,
                } = snap_move(moved, peers, canvas, snapping);
                transform.x += delta.0;
                transform.y += delta.1;
                guides = snap_guides;
            }
            TransformHit::Resize(handle) => {
                let (sx, sy) = handle.axes();
                let (width, height) = (bounds.width(), bounds.height());
                let (new_width, new_height) = if sx != 0.0 && sy != 0.0 {
                    // Project the pointer onto the original diagonal in logical
                    // pixels so non-square canvases preserve the same aspect.
                    let (vx, vy) = (sx * width * canvas.0, sy * height * canvas.1);
                    let factor = (1.0
                        + (dx * canvas.0 * vx + dy * canvas.1 * vy) / (vx * vx + vy * vy))
                        .max(
                            (MIN_SIZE_PX / (width * canvas.0))
                                .max(MIN_SIZE_PX / (height * canvas.1)),
                        );
                    transform.scale *= factor;
                    (width * factor, height * factor)
                } else {
                    let new_width = if sx != 0.0 {
                        (width + sx * dx).max(MIN_SIZE_PX / canvas.0)
                    } else {
                        width
                    };
                    let new_height = if sy != 0.0 {
                        (height + sy * dy).max(MIN_SIZE_PX / canvas.1)
                    } else {
                        height
                    };
                    transform.scale_x *= new_width / width;
                    transform.scale_y *= new_height / height;
                    (new_width, new_height)
                };
                transform.x += sx * (new_width - width) / 2.0;
                transform.y += sy * (new_height - height) / 2.0;

                if let Some(resized) = transform.bounds_for_base(self.base) {
                    let AlignSnap {
                        delta,
                        guides: snap_guides,
                    } = snap_resize(resized, peers, canvas, sx, sy, snapping);
                    // Keep the opposite edge fixed: translate by half the snap
                    // on the driven axis, and absorb the rest into scale.
                    if delta.0 != 0.0 || delta.1 != 0.0 {
                        let before = resized;
                        let mut after = before;
                        if sx < 0.0 {
                            after.left += delta.0;
                        } else if sx > 0.0 {
                            after.right += delta.0;
                        }
                        if sy < 0.0 {
                            after.top += delta.1;
                        } else if sy > 0.0 {
                            after.bottom += delta.1;
                        }
                        if after.left < after.right
                            && after.top < after.bottom
                            && after.width() >= MIN_SIZE_PX / canvas.0
                            && after.height() >= MIN_SIZE_PX / canvas.1
                        {
                            let (cx, cy) = after.center();
                            transform.x = cx - 0.5;
                            transform.y = cy - 0.5;
                            if sx != 0.0 && sy != 0.0 {
                                let factor = (after.width() / before.width())
                                    .min(after.height() / before.height());
                                transform.scale *= factor;
                            } else {
                                if sx != 0.0 {
                                    transform.scale_x *= after.width() / before.width();
                                }
                                if sy != 0.0 {
                                    transform.scale_y *= after.height() / before.height();
                                }
                            }
                            guides = snap_guides;
                        }
                    }
                }
            }
        }
        transform
            .is_valid()
            .then_some(GestureUpdate { transform, guides })
    }
}
