//! Fluid fixed-aspect thumbnail grid (height-for-width): the column
//! count follows the available width, every cell fills its row slot,
//! and the cell height derives from the aspect ratio. Children are
//! allocated exact cell rectangles and never measured, so image
//! widgets cannot leak their intrinsic size into the layout (which is
//! what pinned stock flow grids to a few stretched-apart columns).

use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use gtk4::{glib, Allocation, Orientation, SizeRequestMode, Widget};

mod imp {
    use super::*;
    use std::cell::Cell;

    pub struct TimelineGrid {
        pub aspect: Cell<f64>,
        pub spacing: Cell<i32>,
        pub min_cell_width: Cell<i32>,
    }

    impl Default for TimelineGrid {
        fn default() -> Self {
            Self {
                aspect: Cell::new(16.0 / 9.0),
                spacing: Cell::new(8),
                min_cell_width: Cell::new(240),
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

    pub struct Layout {
        pub cols: i32,
        pub cell_w: i32,
        pub cell_h: i32,
    }

    impl TimelineGrid {
        pub fn layout(&self, width: i32) -> Layout {
            let aspect = self.aspect.get();
            let sp = self.spacing.get();
            let min_w = self.min_cell_width.get();
            let width = width.max(min_w);
            let cols = ((width + sp) / (min_w + sp)).max(1);
            let cell_w = (width - (cols - 1) * sp) / cols;
            let cell_h = (cell_w as f64 / aspect).round() as i32;
            Layout {
                cols,
                cell_w,
                cell_h,
            }
        }

        fn child_count(&self) -> i32 {
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
                    let rows = if layout.cols > 0 {
                        (count + layout.cols - 1) / layout.cols
                    } else {
                        0
                    };
                    let h = if rows > 0 {
                        rows * layout.cell_h + (rows - 1) * self.spacing.get()
                    } else {
                        0
                    };
                    let min_h = if rows > 0 { layout.cell_h } else { 0 };
                    (min_h, h, -1, -1)
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
                    let w = if col == layout.cols - 1 {
                        width - x
                    } else {
                        layout.cell_w
                    };
                    c.size_allocate(&Allocation::new(x, y, w, layout.cell_h), -1);
                    idx += 1;
                }
                child = c.next_sibling();
            }
        }
    }
}

glib::wrapper! {
    /// Fluid grid of fixed-aspect cells: column count follows the
    /// available width and cells fill the row without gaps.
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

    pub fn set_aspect(&self, aspect: f64) {
        let imp = self.imp();
        if (imp.aspect.get() - aspect).abs() > f64::EPSILON {
            imp.aspect.set(aspect);
            self.queue_resize();
        }
    }

    /// The child widget at `index` in layout order, if any.
    pub fn child_at_index(&self, index: i32) -> Option<Widget> {
        let mut n = 0;
        let mut child = self.first_child();
        while let Some(c) = child {
            if c.should_layout() {
                if n == index {
                    return Some(c);
                }
                n += 1;
            }
            child = c.next_sibling();
        }
        None
    }
}
