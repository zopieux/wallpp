use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use gtk4::{Allocation, Orientation, SizeRequestMode, Widget};

mod imp {
    use super::*;
    use std::cell::{Cell, RefCell};

    pub struct AspectBin {
        pub aspect: Cell<f64>,
        pub child: RefCell<Option<Widget>>,
    }

    impl Default for AspectBin {
        fn default() -> Self {
            Self {
                aspect: Cell::new(16.0 / 9.0),
                child: RefCell::new(None),
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
            self.child.borrow_mut().take();
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
            let aspect = self.aspect.get().max(0.01);
            match orientation {
                Orientation::Horizontal => {
                    let min_w = 60;
                    let nat_w = if for_size > 0 {
                        (for_size as f64 * aspect).round() as i32
                    } else {
                        160
                    };
                    (min_w, nat_w, -1, -1)
                }
                _ => {
                    if for_size > 0 {
                        let h = (for_size as f64 / aspect).round() as i32;
                        (h, h, -1, -1)
                    } else {
                        let min_h = (60.0 / aspect).round() as i32;
                        (min_h, min_h, -1, -1)
                    }
                }
            }
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            if let Some(ref child) = *self.child.borrow() {
                if child.should_layout() {
                    let aspect = self.aspect.get().max(0.01);
                    let target_h = (width as f64 / aspect).round() as i32;
                    let (y, h) = if target_h <= height {
                        ((height - target_h) / 2, target_h)
                    } else {
                        (0, height)
                    };
                    child.measure(Orientation::Horizontal, -1);
                    child.size_allocate(&Allocation::new(0, y, width, h), baseline);
                }
            }
        }
    }
}

glib::wrapper! {
    /// Container widget that enforces a fixed aspect ratio for its child:
    /// child width spans the container width, and child height derives from aspect ratio.
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

    pub fn set_child(&self, child: Option<&impl IsA<Widget>>) {
        let imp = self.imp();
        if let Some(old) = imp.child.borrow_mut().take() {
            old.unparent();
        }
        if let Some(new_child) = child {
            new_child.set_parent(self);
            *imp.child.borrow_mut() = Some(new_child.clone().upcast());
        }
        self.queue_resize();
    }

    #[allow(dead_code)]
    pub fn child(&self) -> Option<Widget> {
        self.imp().child.borrow().clone()
    }
}
