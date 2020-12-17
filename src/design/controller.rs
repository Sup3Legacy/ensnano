//! This modules defines the type `design::Controller` that handles the manipulation of the `view`
//! of the design.
//!
//! The `Controller` can be in a state in which the current opperation being applied can varry. In
//! this state, `Conroller` keep track of the old state of the data being modified, in addition to
//! the current state. When the
//! opperation is terminated. The old state of the data is also updated.
use super::{Data, View};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use ultraviolet::{Mat4, Rotor3, Vec3};

type ViewPtr = Rc<RefCell<View>>;
type DataPtr = Arc<Mutex<Data>>;

pub struct Controller {
    /// The view controlled by self
    view: ViewPtr,
    /// The data controlled by self
    data: DataPtr,
    /// A copy of the model_matrix of the view before the current movement
    old_matrix: Mat4,
}

impl Controller {
    pub fn new(view: ViewPtr, data: DataPtr) -> Self {
        Self {
            view,
            data,
            old_matrix: Mat4::identity(),
        }
    }

    /// Translate the whole view of the design
    pub fn translate(&mut self, translation: &DesignTranslation) {
        match translation.target {
            IsometryTarget::Design => self
                .view
                .borrow_mut()
                .set_matrix(self.old_matrix.translated(&translation.translation)),
            IsometryTarget::Grid(g_id) => self
                .data
                .lock()
                .unwrap()
                .translate_grid(g_id as usize, translation.translation),
            IsometryTarget::Helix(h_id, b) => self
                .data
                .lock()
                .unwrap()
                .translate_helix(h_id as usize, translation.translation, b),
        }
    }

    /// Apply a DesignRotation to the view of the design
    pub fn rotate(&mut self, rotation: &DesignRotation) {
        match rotation.target {
            IsometryTarget::Design => {
                // Design are rotated in the worlds coordinates
                let rotor = rotation.rotation.into_matrix().into_homogeneous();

                let origin = rotation.origin;

                let new_matrix = Mat4::from_translation(origin)
                    * rotor
                    * Mat4::from_translation(-origin)
                    * self.old_matrix;
                self.view.borrow_mut().set_matrix(new_matrix);
            }
            IsometryTarget::Helix(n, _) => {
                // Helices are rotated in the model coordinates.
                let origin = self.old_matrix.inversed().transform_point3(rotation.origin);
                let basis = ultraviolet::Mat3::new(
                    self.old_matrix.transform_vec3(Vec3::unit_x()),
                    self.old_matrix.transform_vec3(Vec3::unit_y()),
                    self.old_matrix.transform_vec3(Vec3::unit_z()),
                )
                .into_rotor3();
                self.data.lock().unwrap().rotate_helix_arround(
                    n as usize,
                    rotation.rotation.rotated_by(basis.reversed()),
                    origin,
                )
            }
            IsometryTarget::Grid(n) => {
                let origin = self.old_matrix.inversed().transform_point3(rotation.origin);
                let basis = ultraviolet::Mat3::new(
                    self.old_matrix.transform_vec3(Vec3::unit_x()),
                    self.old_matrix.transform_vec3(Vec3::unit_y()),
                    self.old_matrix.transform_vec3(Vec3::unit_z()),
                )
                .into_rotor3();
                self.data.lock().unwrap().rotate_grid_arround(
                    n as usize,
                    rotation.rotation.rotated_by(basis.reversed()),
                    origin,
                )
            }
        }
    }

    /// Terminate the movement computed by self
    pub fn terminate_movement(&mut self) {
        self.old_matrix = self.view.borrow().model_matrix;
        self.data.lock().unwrap().terminate_movement();
    }
}

/// A rotation on an element of a design.
#[derive(Debug, Clone)]
pub struct DesignRotation {
    pub origin: Vec3,
    pub rotation: Rotor3,
    /// The element of the design on which the rotation will be applied
    pub target: IsometryTarget,
}

/// A translation of an element of a design
#[derive(Clone, Debug)]
pub struct DesignTranslation {
    pub translation: Vec3,
    pub target: IsometryTarget,
}

/// A element on which an isometry must be applied
#[derive(Clone, Debug)]
pub enum IsometryTarget {
    /// The view of the whole design
    Design,
    /// An helix of the design
    Helix(u32, bool),
    /// A grid of the desgin
    Grid(u32),
}
