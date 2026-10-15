//! Single-child bin with a fixed aspect ratio (height-for-width):
//! the height derives from the allocated width exactly like the
//! timeline grid's cells, and the child fills the whole allocation.
//! The child is never measured, so a GtkPicture's intrinsic image
//! size cannot leak into the parent's layout (stock containers
//! forward it, which stretched the landing inspector's preview to
//! whatever aspect the wallpaper file happened to have instead of
//! the monitor's).

use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use gtk4::{glib, Allocation, Orientation, SizeRequestMode, Widget};

mod imp {
    use super::*;
    use std::cell::Cell;

    pub struct AspectBin {
        pub aspect: Cell<f64>,
    }

    impl Default for AspectBin {
        fn default() -> Self {
            Self {
                aspect: Cell::new(16.0 / 9.0),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for AspectBin {
        const NAME: &'static str = "WallppAspectBin";
        type Type = super::AspectBin;
        type ParentType = Widget;
    }

    impl ObjectImpl for AspectBin {
        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for AspectBin {
        fn request_mode(&self) -> SizeRequestMode {
            SizeRequestMode::HeightForWidth
        }

        fn measure(&self, orientation: Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            match orientation {
                // Never measure the child: its natural size (a
                // picture's intrinsic image size) must not widen
                // the inspector beyond its share of the window.
                Orientation::Horizontal => (0, 0, -1, -1),
                _ => {
                    let width = if for_size > 0 { for_size } else { 640 };
                    let h = (width as f64 / self.aspect.get()).round() as i32;
                    (h, h, -1, -1)
                }
            }
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            if let Some(child) = self.obj().first_child() {
                child.size_allocate(&Allocation::new(0, 0, width, height), baseline);
            }
        }
    }
}

glib::wrapper! {
    /// A bin whose height follows its width at a fixed aspect ratio,
    /// so its child (cover-fit) shows exactly the monitor's aspect.
    pub struct AspectBin(ObjectSubclass<imp::AspectBin>)
        @extends Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

impl AspectBin {
    pub fn new(aspect: f64) -> Self {
        let bin: Self = glib::Object::new();
        bin.imp().aspect.set(aspect);
        bin
    }

    pub fn set_child(&self, child: &impl IsA<Widget>) {
        child.set_parent(self);
    }

    pub fn set_aspect(&self, aspect: f64) {
        let imp = self.imp();
        if (imp.aspect.get() - aspect).abs() > f64::EPSILON {
            imp.aspect.set(aspect);
            self.queue_resize();
        }
    }
}
