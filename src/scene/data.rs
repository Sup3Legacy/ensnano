use std::rc::Rc;
use std::cell::RefCell;
use std::sync::{Arc, Mutex};
use std::collections::HashSet;
use super::{View, ViewUpdate};

use ultraviolet::{Rotor3, Vec3};

use crate::utils::instance::Instance;
use crate::design::{Design, Referential};

type ViewPtr = Rc<RefCell<View>>;

/// A module that handles the instantiation of designs as 3D geometric objects
mod design3d;
use design3d::Design3D;

pub struct Data {
    view: ViewPtr,
    designs: Vec<Design3D>,
    selected: Vec<(u32, u32)>,
    candidates: Vec<(u32, u32)>,
    selection_mode: SelectionMode,
}

impl Data {

    pub fn new(view: ViewPtr) -> Self {
        Self {
            view,
            designs: Vec::new(),
            selected: Vec::new(),
            candidates: Vec::new(),
            selection_mode: SelectionMode::default(),
        }
    }

    pub fn add_design(&mut self, design: Arc<Mutex<Design>>) {
        self.designs
            .push(Design3D::new(design));
        self.notify_instance_update();
        self.notify_selection_update();
        self.notify_candidate_update();
        self.notify_matrices_update();
    }

    pub fn clear_designs(&mut self) {
        self.designs = Vec::new();
        self.selected = Vec::new();
        self.candidates = Vec::new();
        self.notify_selection_update();
        self.notify_candidate_update();
        self.notify_instance_update();
    }

    pub fn get_selected_designs(&self) -> HashSet<u32> {
        self.selected.iter().map(|x| x.0).collect()
    }

    pub fn get_candidate_designs(&self) -> HashSet<u32> {
        self.candidates.iter().map(|x| x.0).collect()
    }

    /// Return the instances of selected spheres
    pub fn get_selected_spheres(&self) -> Rc<Vec<Instance>> {
        let mut ret = Vec::with_capacity(self.selected.len());
        match self.selection_mode {
            SelectionMode::Nucleotide => {
                for (d_id, id) in self.selected.iter() {
                    let d_id = *d_id as usize;
                    if self.designs[d_id].is_nucl(*id) {
                        ret.push(self.designs[d_id].make_instance(*id))
                    }
                }
            }
            SelectionMode::Design => {
                for d_id in self.get_selected_designs().iter() {
                    for sphere in self.designs[*d_id as usize].get_spheres().iter() {
                        ret.push(*sphere)
                    }
                }
            }
        }
        Rc::new(ret)
    }

    /// Return the instances of selected tubes
    pub fn get_selected_tubes(&self) -> Rc<Vec<Instance>> {
        let mut ret = Vec::with_capacity(self.selected.len());
        match self.selection_mode {
            SelectionMode::Nucleotide => {
                for (d_id, id) in self.selected.iter() {
                    let d_id = *d_id as usize;
                    if self.designs[d_id].is_bound(*id) {
                        ret.push(self.designs[d_id].make_instance(*id))
                    }
                }
            }
            SelectionMode::Design => {
                for d_id in self.get_selected_designs().iter() {
                    for tube in self.designs[*d_id as usize].get_tubes().iter() {
                        ret.push(*tube)
                    }
                }
            }
        }
        Rc::new(ret)
    }

    /// Return the instances of candidate spheres
    pub fn get_candidate_spheres(&self) -> Rc<Vec<Instance>> {
        let mut ret = Vec::with_capacity(self.selected.len());
        match self.selection_mode {
            SelectionMode::Nucleotide => {
                for (d_id, id) in self.candidates.iter() {
                    let d_id = *d_id as usize;
                    if self.designs[d_id].is_nucl(*id) {
                        ret.push(self.designs[d_id].make_instance(*id))
                    }
                }
            }
            SelectionMode::Design => {
                for d_id in self.get_candidate_designs().iter() {
                    for sphere in self.designs[*d_id as usize].get_spheres().iter() {
                        ret.push(*sphere)
                    }
                }
            }
        }
        Rc::new(ret)
    }

    /// Return the instances of candidate tubes
    pub fn get_candidate_tubes(&self) -> Rc<Vec<Instance>> {
        let mut ret = Vec::with_capacity(self.selected.len());
        match self.selection_mode {
            SelectionMode::Nucleotide => {
                for (d_id, id) in self.candidates.iter() {
                    let d_id = *d_id as usize;
                    if self.designs[d_id].is_bound(*id) {
                        ret.push(self.designs[d_id].make_instance(*id))
                    }
                }
            }
            SelectionMode::Design => {
                for d_id in self.get_candidate_designs().iter() {
                    for tube in self.designs[*d_id as usize].get_tubes().iter() {
                        ret.push(*tube)
                    }
                }
            }
        }
        Rc::new(ret)
    }

    pub fn get_element_position(&self, design_id: u32, element_id: u32, referential: Referential) -> Vec3 {
        self.designs[design_id as usize].get_element_position(element_id, referential).unwrap()
    }

    pub fn get_selected_position(&self) -> Option<Vec3> {
        let (desgin_id, element_id) = self.selected.get(0)?;
        Some(self.get_element_position(*desgin_id, *element_id, Referential::World))
    }

    pub fn set_selection(&mut self, design_id: u32, element_id: u32) {
        self.selected = vec![(design_id, element_id)];
    }

    pub fn reset_selection(&mut self) {
        self.selected = Vec::new();
    }

    pub fn notify_selection_update(&mut self) {
        self.view.borrow_mut().update(ViewUpdate::SelectedTubes(self.get_selected_tubes()));
        self.view.borrow_mut().update(ViewUpdate::SelectedSpheres(self.get_selected_spheres()));
    }

    pub fn set_candidate(&mut self, design_id: u32, element_id: u32) {
        self.candidates = vec![(design_id, element_id)];
    }

    pub fn reset_candidate(&mut self) {
        self.candidates = Vec::new();
    }

    pub fn notify_candidate_update(&mut self) {
        self.view.borrow_mut().update(ViewUpdate::CandidateTubes(self.get_candidate_tubes()));
        self.view.borrow_mut().update(ViewUpdate::CandidateSpheres(self.get_candidate_spheres()));
    }

    pub fn notify_instance_update(&mut self) {
        let mut spheres = Vec::with_capacity(self.get_number_spheres());
        let mut tubes = Vec::with_capacity(self.get_number_tubes());

        for design in self.designs.iter() {
            for sphere in design.get_spheres().iter() {
                spheres.push(*sphere);
            }
            for tube in design.get_tubes().iter() {
                tubes.push(*tube);
            }
        }
        self.view.borrow_mut().update(ViewUpdate::Tubes(Rc::new(tubes)));
        self.view.borrow_mut().update(ViewUpdate::Spheres(Rc::new(spheres)));
    }

    pub fn notify_matrices_update(&mut self) {
        let mut matrices = Vec::new();
        for design in self.designs.iter() {
            matrices.push(design.get_model_matrix());
        }
        self.view.borrow_mut().update(ViewUpdate::ModelMatrices(matrices));
    }

    pub fn get_fitting_camera(&self, ratio: f32, fovy: f32) -> Option<(Vec3, Rotor3)> {
        let design = self.designs.get(0)?;
        Some(design.get_fitting_camera(ratio, fovy))
    }

    pub fn get_middle_point(&self, design_id: u32) -> Vec3 {
        self.designs[design_id as usize].middle_point()
    }

    fn get_number_spheres(&self) -> usize {
        self.designs.iter().map(|d| d.get_spheres().len()).sum()
    }

    fn get_number_tubes(&self) -> usize {
        self.designs.iter().map(|d| d.get_tubes().len()).sum()
    }

    pub fn toggle_selection_mode(&mut self) {
        self.selection_mode = match self.selection_mode {
            SelectionMode::Design => SelectionMode::Nucleotide,
            SelectionMode::Nucleotide => SelectionMode::Design,
        }
    }

    pub fn change_selection_mode(&mut self, selection_mode: SelectionMode) {
        self.selection_mode = selection_mode;
    }

}

fn last_two_bytes(x: u32) -> u32 {
    (x & 0xFF000000) >> 24
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionMode {
    Nucleotide,
    Design,
}

impl Default for SelectionMode {
    fn default() -> Self {
        SelectionMode::Nucleotide
    }
}

impl std::fmt::Display for SelectionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                SelectionMode::Design => "Design",
                SelectionMode::Nucleotide => "Nucleotide",
            }
        )
    }
}

impl SelectionMode {
    pub const ALL: [SelectionMode; 2] = [
        SelectionMode::Nucleotide,
        SelectionMode::Design
    ];
}

