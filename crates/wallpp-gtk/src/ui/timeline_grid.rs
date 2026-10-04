use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use gtk4::{Allocation, Orientation, SizeRequestMode, Widget};

mod imp {
    use super::*;
    use std::cell::Cell;

    pub struct TimelineGrid {
        pub aspect: Cell<f64>,
        pub spacing: Cell<i32>,
        pub min_cell_width: Cell<i32>,
        pub target_rows: Cell<f64>,
        pub viewport_height: Cell<i32>,
    }

    impl Default for TimelineGrid {
        fn default() -> Self {
            Self {
                aspect: Cell::new(16.0 / 9.0),
                spacing: Cell::new(8),
                min_cell_width: Cell::new(180),
                target_rows: Cell::new(4.0),
                viewport_height: Cell::new(0),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for TimelineGrid {
        const NAME: &'static str = "WallppTimelineGrid";
        type Type = super::TimelineGrid;
        type ParentType = Widget;
    }

    impl ObjectImpl for TimelineGrid {
        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for TimelineGrid {
        fn request_mode(&self) -> SizeRequestMode {
            SizeRequestMode::HeightForWidth
        }

        fn measure(&self, orientation: Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let min_w = self.min_cell_width.get();
            match orientation {
                Orientation::Horizontal => (min_w, min_w, -1, -1),
                _ => {
                    let width = if for_size > 0 { for_size } else { min_w };
                    let layout = self.layout(width);
                    let count = self.child_count();
                    let rows = (count + layout.cols - 1) / layout.cols;
                    let h = if rows > 0 {
                        rows * layout.cell_h + (rows - 1) * self.spacing.get()
                    } else {
                        0
                    };
                    (h, h, -1, -1)
                }
            }
        }

        fn size_allocate(&self, width: i32, _height: i32, _baseline: i32) {
            let layout = self.layout(width);
            let sp = self.spacing.get();
            let mut idx = 0;
            let mut child = self.obj().first_child();
            while let Some(c) = child {
                if c.should_layout() {
                    let col = idx % layout.cols;
                    let row = idx / layout.cols;
                    let x = col * (layout.cell_w + sp);
                    let y = row * (layout.cell_h + sp);
                    let w = if col == layout.cols - 1 { width - x } else { layout.cell_w };
                    c.measure(Orientation::Horizontal, -1);
                    c.size_allocate(&Allocation::new(x, y, w, layout.cell_h), -1);
                    idx += 1;
                }
                child = c.next_sibling();
            }
        }
    }

    pub struct Layout {
        pub cols: i32,
        pub cell_w: i32,
        pub cell_h: i32,
    }

    impl TimelineGrid {
        pub fn child_count(&self) -> i32 {
            let mut n = 0;
            let mut child = self.obj().first_child();
            while let Some(c) = child {
                if c.should_layout() {
                    n += 1;
                }
                child = c.next_sibling();
            }
            n
        }

        /// Pick the column count whose cell height best fits `target_rows` rows in the
        /// visible viewport, bounded by the minimum absolute cell width.
        pub fn layout(&self, width: i32) -> Layout {
            let aspect = self.aspect.get();
            let sp = self.spacing.get();
            let min_w = self.min_cell_width.get();
            let width = width.max(min_w);

            let max_cols = ((width + sp) / (min_w + sp)).max(1);
            let vh = self.viewport_height.get();
            let ideal_cols = if vh > 0 {
                let target_h = vh as f64 / self.target_rows.get();
                let target_w = target_h * aspect;
                ((width + sp) as f64 / (target_w + sp as f64)).round() as i32
            } else {
                max_cols
            };
            let cols = ideal_cols.clamp(1, max_cols);

            let cell_w = (width - (cols - 1) * sp) / cols;
            let cell_h = (cell_w as f64 / aspect).round() as i32;
            Layout { cols, cell_w, cell_h }
        }
    }
}

glib::wrapper! {
    /// Fluid grid of fixed-aspect cells: column count follows the available width,
    /// cell width fills the row, and cell height derives from the aspect ratio.
    pub struct TimelineGrid(ObjectSubclass<imp::TimelineGrid>)
        @extends Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

impl TimelineGrid {
    pub fn new(aspect: f64) -> Self {
        let grid: Self = glib::Object::new();
        grid.imp().aspect.set(aspect);
        grid
    }

    pub fn append(&self, child: &impl IsA<Widget>) {
        child.set_parent(self);
    }

    pub fn clear(&self) {
        while let Some(child) = self.first_child() {
            child.unparent();
        }
    }

    pub fn set_viewport_height(&self, height: i32) {
        let imp = self.imp();
        if imp.viewport_height.get() != height {
            imp.viewport_height.set(height);
            self.queue_resize();
        }
    }

    /// Vertical offset and height of the cell at `index` for the current allocation.
    pub fn cell_extent(&self, index: usize) -> Option<(i32, i32)> {
        let width = self.width();
        if width <= 0 {
            return None;
        }
        let imp = self.imp();
        let layout = imp.layout(width);
        let row = index as i32 / layout.cols;
        Some((row * (layout.cell_h + imp.spacing.get()), layout.cell_h))
    }
}
