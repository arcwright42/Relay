/// Global, logical coordinates with a top-left origin. No retina pixel scaling.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub fn relative_to(self, screen: Self) -> Self {
        Self {
            x: self.x - screen.x,
            y: self.y - screen.y,
            ..self
        }
    }

    pub fn fit(self, screen: Self) -> Self {
        let margin = 8.;
        let width = self.width.min((screen.width - margin * 2.).max(1.));
        let height = self.height.min((screen.height - margin * 2.).max(1.));
        Self {
            x: self.x.clamp(
                screen.x + margin,
                (screen.x + screen.width - width - margin).max(screen.x + margin),
            ),
            y: self.y.clamp(
                screen.y + margin,
                (screen.y + screen.height - height - margin).max(screen.y + margin),
            ),
            width,
            height,
        }
    }

    pub fn near_pointer(pointer: (f64, f64), width: f64, height: f64, screen: Self) -> Self {
        let gap = 12.;
        let below = pointer.1 + gap;
        let y = if below + height <= screen.y + screen.height - 8. {
            below
        } else {
            pointer.1 - height - gap
        };
        Self {
            x: pointer.0 + 5.,
            y,
            width,
            height,
        }
        .fit(screen)
    }
}

#[cfg(test)]
mod tests {
    use super::Rect;
    const SCREEN: Rect = Rect {
        x: 0.,
        y: 25.,
        width: 1440.,
        height: 825.,
    };
    #[test]
    fn display_local_coordinates_remove_the_real_nonzero_origin_once() {
        for screen in [
            Rect {
                x: 1440.,
                y: 200.,
                width: 1920.,
                height: 1080.,
            },
            Rect {
                x: -1920.,
                y: -500.,
                width: 1920.,
                height: 1080.,
            },
            Rect {
                x: 100.,
                y: 900.,
                width: 1920.,
                height: 1080.,
            },
        ] {
            let placed = Rect::near_pointer((screen.x + 500., screen.y + 400.), 640., 54., screen);
            let local = placed.relative_to(screen);
            assert_eq!((local.x, local.y), (505., 412.));
            assert_eq!(
                (local.x + screen.x, local.y + screen.y),
                (placed.x, placed.y)
            );
        }
    }
    #[test]
    fn toolbar_does_not_reserve_future_result_height() {
        let r = Rect::near_pointer((500., 700.), 640., 54., SCREEN);
        assert_eq!((r.x, r.y), (505., 712.));
    }
    #[test]
    fn bottom_right_flips_above_and_fits_horizontally() {
        let r = Rect::near_pointer((1400., 825.), 640., 54., SCREEN);
        assert_eq!((r.x, r.y), (792., 759.));
    }
    #[test]
    fn result_growth_stays_on_original_screen() {
        let r = Rect {
            x: 505.,
            y: 712.,
            width: 520.,
            height: 580.,
        }
        .fit(SCREEN);
        assert_eq!((r.x, r.y), (505., 262.));
    }
    #[test]
    fn negative_secondary_display_and_dock_are_respected() {
        let screen = Rect {
            x: -1920.,
            y: -800.,
            width: 1860.,
            height: 1050.,
        };
        let r = Rect::near_pointer((-100., 225.), 640., 54., screen);
        assert_eq!((r.x, r.y), (-708., 159.));
        let tiny = Rect {
            x: 0.,
            y: 0.,
            width: 300.,
            height: 250.,
        };
        assert_eq!(
            r.fit(tiny),
            Rect {
                x: 8.,
                y: 159.,
                width: 284.,
                height: 54.
            }
        );
    }
}
