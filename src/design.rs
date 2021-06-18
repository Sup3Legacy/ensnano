/*
ENSnano, a 3d graphical application for DNA nanostructures.
    Copyright (C) 2021  Nicolas Levy <nicolaspierrelevy@gmail.com> and Nicolas Schabanel <nicolas.schabanel@ens-lyon.fr>

    This program is free software: you can redistribute it and/or modify
    it under the terms of the GNU General Public License as published by
    the Free Software Foundation, either version 3 of the License, or
    (at your option) any later version.

    This program is distributed in the hope that it will be useful,
    but WITHOUT ANY WARRANTY; without even the implied warranty of
    MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
    GNU General Public License for more details.

    You should have received a copy of the GNU General Public License
    along with this program.  If not, see <https://www.gnu.org/licenses/>.
*/
//! This modules defines the type [`Design`](Design) which offers an interface to a DNA nanostructure design.
use crate::gui::SimulationRequest;
use ahash::RandomState;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use ultraviolet::{Mat4, Vec3};

use crate::mediator;
use mediator::{AppNotification, Selection, UndoableOp};

mod controller;
mod data;
mod operation;
pub mod utils;
mod view;
use crate::scene::GridInstance;
use controller::Controller;
pub use controller::{DesignRotation, DesignTranslation, IsometryTarget};
use data::Data;
pub use data::*;
use ensnano_organizer::OrganizerTree;
pub use utils::*;
use view::View;

pub struct Design {
    view: Arc<Mutex<View>>,
    #[allow(dead_code)]
    controller: Controller,
    data: Arc<Mutex<Data>>,
    id: usize,
}

impl Design {
    #[allow(dead_code)]
    pub fn new(id: usize) -> Self {
        let view = Arc::new(Mutex::new(View::new()));
        let data = Arc::new(Mutex::new(Data::new()));
        let controller = Controller::new(view.clone(), data.clone());
        Self {
            view,
            data,
            controller,
            id,
        }
    }

    /// Create a new design by reading a file. At the moment only codenano format is supported
    pub fn new_with_path(id: usize, path: &PathBuf) -> Option<Self> {
        let view = Arc::new(Mutex::new(View::new()));
        let data = Arc::new(Mutex::new(Data::new_with_path(path)?));
        let controller = Controller::new(view.clone(), data.clone());
        Some(Self {
            view,
            data,
            controller,
            id,
        })
    }

    /// `true` if the view has been updated since the last time this function was called
    pub fn view_was_updated(&self) -> Option<DesignNotification> {
        if self.view.lock().unwrap().was_updated() {
            let notification = DesignNotification {
                content: DesignNotificationContent::ModelChanged(self.get_model_matrix()),
                design_id: self.id as usize,
            };
            Some(notification)
        } else {
            None
        }
    }

    /// Return a notification to send to the observer if the data was changed.
    pub fn data_was_updated(&self) -> Option<DesignNotification> {
        if self.data.lock().unwrap().view_need_reset() {
            let notification = DesignNotification {
                content: DesignNotificationContent::ViewNeedReset,
                design_id: self.id as usize,
            };
            Some(notification)
        } else if self.data.lock().unwrap().was_updated() {
            let notification = DesignNotification {
                content: DesignNotificationContent::InstanceChanged,
                design_id: self.id as usize,
            };
            Some(notification)
        } else {
            None
        }
    }

    /// Return the model matrix used to display the design
    pub fn get_model_matrix(&self) -> Mat4 {
        self.view.lock().unwrap().get_model_matrix()
    }

    /// Translate the representation of self
    fn apply_translation(&mut self, translation: &DesignTranslation) -> bool {
        self.controller.translate(translation)
    }

    /// Rotate the representation of self arround `origin`
    pub fn apply_rotation(&mut self, rotation: &DesignRotation) {
        self.controller.rotate(rotation);
    }

    /// Terminate the movement performed by self.
    pub fn terminate_movement(&mut self) {
        self.controller.terminate_movement()
    }

    /// Get the position of an item of self in a given rerential
    pub fn get_element_position(&self, id: u32, referential: Referential) -> Option<Vec3> {
        if referential.is_world() {
            self.data
                .lock()
                .unwrap()
                .get_element_position(id)
                .map(|x| self.view.lock().unwrap().model_matrix.transform_point3(x))
        } else {
            self.data.lock().unwrap().get_element_position(id)
        }
    }

    /// Get the position of an item of self in a given referential
    pub fn get_element_axis_position(&self, id: u32, referential: Referential) -> Option<Vec3> {
        if referential.is_world() {
            self.data
                .lock()
                .unwrap()
                .get_element_axis_position(id)
                .map(|x| self.view.lock().unwrap().model_matrix.transform_point3(x))
        } else {
            self.data.lock().unwrap().get_element_axis_position(id)
        }
    }

    /// Get the position of a nucleotide in a given referential. Eventually project the nucleotide
    /// on the it's helix's axis.
    pub fn get_helix_nucl(
        &self,
        nucl: Nucl,
        referential: Referential,
        on_axis: bool,
    ) -> Option<Vec3> {
        if referential.is_world() {
            self.data
                .lock()
                .unwrap()
                .get_helix_nucl(nucl, on_axis)
                .map(|x| self.view.lock().unwrap().model_matrix.transform_point3(x))
        } else {
            self.data.lock().unwrap().get_helix_nucl(nucl, on_axis)
        }
    }

    /// Return the `ObjectType` of an element
    pub fn get_object_type(&self, id: u32) -> Option<ObjectType> {
        self.data.lock().unwrap().get_object_type(id)
    }

    /// Return the color of an element
    pub fn get_color(&self, id: u32) -> Option<u32> {
        self.data.lock().unwrap().get_color(id)
    }

    /// Return all identifier of nucleotides
    pub fn get_all_nucl_ids(&self) -> Vec<u32> {
        self.data.lock().unwrap().get_all_nucl_ids().collect()
    }

    pub fn get_all_visible_nucl_ids(&self) -> Vec<u32> {
        self.data.lock().unwrap().get_all_visible_nucl_ids()
    }

    pub fn get_all_visible_bound_ids(&self) -> Vec<u32> {
        self.data.lock().unwrap().get_all_visible_bound_ids()
    }

    pub fn get_visibility_helix(&self, h_id: usize) -> Option<bool> {
        self.data.lock().unwrap().get_visibility_helix(h_id)
    }

    pub fn set_visibility_helix(&mut self, h_id: usize, visibility: bool) {
        self.data
            .lock()
            .unwrap()
            .set_visibility_helix(h_id, visibility)
    }

    /// Return all identifer of bounds
    pub fn get_all_bound_ids(&self) -> Vec<u32> {
        self.data.lock().unwrap().get_all_bound_ids().collect()
    }

    pub fn apply_operation(&mut self, operation: UndoableOp) -> OperationResult {
        match operation {
            UndoableOp::Rotation(rotation) => self.apply_rotation(&rotation),
            UndoableOp::Translation(translation) => {
                if self.apply_translation(&translation) {
                    return OperationResult::UndoableChange;
                } else {
                    return OperationResult::NoChange;
                }
            }
            UndoableOp::MakeAllGrids => self.data.lock().unwrap().create_grids(),
            UndoableOp::AddGridHelix(GridHelixDescriptor { grid_id, x, y }, position, length) => {
                self.data
                    .lock()
                    .unwrap()
                    .build_helix_grid(grid_id, x, y, position, length)
            }
            UndoableOp::RmGridHelix(GridHelixDescriptor { grid_id, x, y }, position, length) => {
                if length > 0 {
                    self.data
                        .lock()
                        .unwrap()
                        .rm_full_helix_grid(grid_id, x, y, position)
                }
                self.data.lock().unwrap().rm_helix_grid(grid_id, x, y)
            }
            UndoableOp::RmStrand {
                strand,
                strand_id,
                undo,
            } => {
                let init = self.data.lock().unwrap().get_strand_state();
                self.data
                    .lock()
                    .unwrap()
                    .undoable_rm_strand(strand, strand_id, undo);
                let after = self.data.lock().unwrap().get_strand_state();
                return OperationResult::BigChange(init, after);
            }
            UndoableOp::RmGrid => self.data.lock().unwrap().delete_last_grid(),
            UndoableOp::AddGrid(grid_descriptor) => {
                self.data.lock().unwrap().add_grid(grid_descriptor);
            }
            UndoableOp::ResetBuilder(builder) => {
                let mut builder = builder.clone();
                builder.reset();
                if builder.created_de_novo() {
                    let nucl = builder.get_moving_end_nucl();
                    self.data.lock().unwrap().rm_strand_containing_nucl(&nucl);
                }
            }
            UndoableOp::MoveBuilder(builder, remake) => {
                let mut builder = builder.clone();
                if let Some((s_id, color)) = remake {
                    let nucl = builder.get_initial_nucl();
                    self.data.lock().unwrap().remake_strand(nucl, s_id, color);
                }
                builder.update();
            }
            UndoableOp::RawHelixCreation {
                helix,
                h_id,
                delete,
            } => {
                if delete {
                    self.data.lock().unwrap().remove_helix(h_id)
                } else {
                    self.data.lock().unwrap().add_helix(&helix, h_id)
                }
            }
            UndoableOp::Cut {
                nucl,
                strand,
                undo,
                s_id,
            } => {
                let init = self.data.lock().unwrap().get_strand_state();
                if undo {
                    self.data.lock().unwrap().undo_split(strand, s_id)
                } else {
                    self.data.lock().unwrap().split_strand(&nucl, None);
                }
                let after = self.data.lock().unwrap().get_strand_state();
                return OperationResult::BigChange(init, after);
            }
            UndoableOp::Xover {
                strand_5prime,
                strand_3prime,
                prime5_id,
                prime3_id,
                undo,
            } => {
                let init = self.data.lock().unwrap().get_strand_state();
                if prime5_id == prime3_id {
                    self.data.lock().unwrap().make_cycle(prime5_id, !undo)
                } else {
                    if undo {
                        self.data.lock().unwrap().undo_merge(
                            strand_5prime,
                            strand_3prime,
                            prime5_id,
                            prime3_id,
                        )
                    } else {
                        self.data
                            .lock()
                            .unwrap()
                            .merge_strands(prime5_id, prime3_id)
                    }
                }
                let after = self.data.lock().unwrap().get_strand_state();
                return OperationResult::BigChange(init, after);
            }
            UndoableOp::CrossCut {
                source_strand,
                target_strand,
                source_id,
                target_id,
                target_3prime,
                nucl,
                undo,
            } => {
                println!("Cross cut {} {}", source_id, target_id);
                let init = self.data.lock().unwrap().get_strand_state();
                if undo {
                    self.data.lock().unwrap().undo_cross_cut(
                        source_strand,
                        target_strand,
                        source_id,
                        target_id,
                    )
                } else {
                    self.data
                        .lock()
                        .unwrap()
                        .cross_cut(source_id, target_id, nucl, target_3prime)
                }
                let after = self.data.lock().unwrap().get_strand_state();
                return OperationResult::BigChange(init, after);
            }
            UndoableOp::NewHyperboloid {
                position,
                orientation,
                hyperboloid,
            } => {
                self.data
                    .lock()
                    .unwrap()
                    .add_hyperboloid(position, orientation, hyperboloid);
            }
            UndoableOp::ClearHyperboloid => self.data.lock().unwrap().clear_hyperboloid(),
            UndoableOp::NewStrandState(state) => self.data.lock().unwrap().new_strand_state(state),
            UndoableOp::ResetCopyPaste => self.data.lock().unwrap().reset_copy_paste(),
            UndoableOp::UndoGridSimulation(initial_state) => self
                .data
                .lock()
                .unwrap()
                .undo_grid_simulation(initial_state),
            UndoableOp::UndoHelixSimulation(initial_state) => self
                .data
                .lock()
                .unwrap()
                .undo_helix_simulation(initial_state),
        }
        OperationResult::UndoableChange
    }

    /// Notify the design of a notification. This is how applications communicate their
    /// modification request to the design
    pub fn on_notify(&mut self, notification: AppNotification) {
        match notification {
            AppNotification::MovementEnded => self.terminate_movement(),
            AppNotification::ResetCopyPaste => self.data.lock().unwrap().reset_copy_paste(),
            AppNotification::MakeGrids(h_ids) => {
                self.data.lock().unwrap().make_grid_from_helices(&h_ids)
            }
        }
    }

    /// The identifier of the design
    pub fn get_id(&self) -> usize {
        self.id
    }

    /// Return the identifier of the strand on which an element lies
    pub fn get_strand(&self, element_id: u32) -> Option<usize> {
        self.data.lock().unwrap().get_strand_of_element(element_id)
    }

    /// Return the identifier of the helix on which an element lies
    pub fn get_helix(&self, element_id: u32) -> Option<usize> {
        self.data.lock().unwrap().get_helix_of_element(element_id)
    }

    /// Return all the identifier of the elements that lie on a strand
    pub fn get_strand_elements(&self, strand_id: usize) -> Vec<u32> {
        self.data.lock().unwrap().get_strand_elements(strand_id)
    }

    pub fn get_strand_length(&self, strand_id: usize) -> Option<usize> {
        self.data.lock().unwrap().get_strand_length(strand_id)
    }

    /// Return all the identifier of the elements that lie on an helix
    pub fn get_helix_elements(&self, helix_id: usize) -> Vec<u32> {
        self.data.lock().unwrap().get_helix_elements(helix_id)
    }

    /// Save the design in icednano format
    pub fn save_to(&self, path: &PathBuf) {
        let result = self.data.lock().unwrap().request_save(path);
        if result.is_err() {
            let text = format!("Could not save_file {:?}", result);
            crate::utils::message(text.into(), rfd::MessageLevel::Error);
        }
    }

    /// Change the collor of a strand
    pub fn change_strand_color(&mut self, strand_id: usize, color: u32) {
        self.data
            .lock()
            .unwrap()
            .change_strand_color(strand_id, color);
    }

    /// Change the sequence of a strand
    pub fn change_strand_sequence(&mut self, strand_id: usize, sequence: String) {
        self.data
            .lock()
            .unwrap()
            .change_strand_sequence(strand_id, sequence);
    }

    pub fn get_strand_color(&self, strand_id: usize) -> Option<u32> {
        self.data.lock().unwrap().get_strand_color(strand_id)
    }

    pub fn get_strand_sequence(&self, strand_id: usize) -> Option<String> {
        self.data.lock().unwrap().get_strand_sequence(strand_id)
    }

    /// Get the basis of the model in the world's coordinates
    pub fn get_basis(&self) -> ultraviolet::Rotor3 {
        let mat4 = self.view.lock().unwrap().get_model_matrix();
        let mat3 = ultraviolet::Mat3::new(
            mat4.transform_vec3(Vec3::unit_x()),
            mat4.transform_vec3(Vec3::unit_y()),
            mat4.transform_vec3(Vec3::unit_z()),
        );
        mat3.into_rotor3()
    }

    /// Return the basis of an helix in the world's coordinates
    pub fn get_helix_basis(&self, h_id: u32) -> Option<ultraviolet::Rotor3> {
        self.data
            .lock()
            .unwrap()
            .get_helix_basis(h_id as usize)
            .map(|r| self.get_basis() * r)
    }

    /// Return the identifier of the 5' end of the strand on which an element lies.
    pub fn get_element_5prime(&self, element: u32) -> Option<u32> {
        let strand = self.get_strand(element)?;
        self.data.lock().unwrap().get_5prime(strand)
    }

    /// Return the identifier of the 3' end of the strand on which an element lies.
    pub fn get_element_3prime(&self, element: u32) -> Option<u32> {
        let strand = self.get_strand(element)?;
        self.data.lock().unwrap().get_3prime(strand)
    }

    /// Return a `StrandBuilder` with moving end `nucl` if possibile (see
    /// [`Data::get_strand_builder`](data::Data::get_strand_builder)).
    pub fn get_builder(&self, nucl: Nucl, stick: bool) -> Option<StrandBuilder> {
        self.data
            .lock()
            .unwrap()
            .get_strand_builder(nucl, stick)
            .map(|b| {
                b.transformed(&self.view.lock().unwrap().get_model_matrix())
                    .given_data(self.data.clone(), self.id as u32)
            })
    }

    /// Return a `StrandBuilder` whose moving end is given by an element, if possible ( see
    /// [`Data::get_strand_builder`](data::Data::get_strand_builder) )
    pub fn get_builder_element(&mut self, element_id: u32, stick: bool) -> Option<StrandBuilder> {
        let nucl = self.data.lock().unwrap().get_nucl(element_id)?;
        self.get_builder(nucl, stick)
    }

    /// If element_id is the identifier of a nucleotide, return the position on which the
    /// nucleotide's symbols must be displayed
    pub fn get_symbol_position(&self, element_id: u32) -> Option<Vec3> {
        self.data.lock().unwrap().get_symbol_position(element_id)
    }

    /// If element_id is the identifier of a nucleotide, return the eventual corresponding
    /// symbols
    pub fn get_symbol(&self, element_id: u32) -> Option<char> {
        self.data.lock().unwrap().get_symbol(element_id)
    }

    pub fn get_strand_points(&self, s_id: usize) -> Option<Vec<Nucl>> {
        self.data.lock().unwrap().get_strand_points(s_id)
    }

    pub fn get_copy_points(&self) -> Vec<Vec<Nucl>> {
        self.data.lock().unwrap().get_copy_points()
    }

    pub fn get_identifier_nucl(&self, nucl: &Nucl) -> Option<u32> {
        self.data.lock().unwrap().get_identifier_nucl(nucl)
    }

    pub fn get_identifier_bound(&self, n1: &Nucl, n2: &Nucl) -> Option<u32> {
        self.data.lock().unwrap().get_identifier_bound(n1, n2)
    }

    pub fn merge_strands(&mut self, prime5: usize, prime3: usize) {
        self.data.lock().unwrap().merge_strands(prime5, prime3)
    }

    pub fn get_all_strand_ids(&self) -> Vec<usize> {
        self.data.lock().unwrap().get_all_strand_ids()
    }

    pub fn prime3_of(&self, nucl: Nucl) -> Option<usize> {
        self.data.lock().unwrap().prime3_of(&nucl)
    }

    pub fn prime5_of(&self, nucl: Nucl) -> Option<usize> {
        self.data.lock().unwrap().prime5_of(&nucl)
    }

    pub fn split_strand(&self, nucl: Nucl) {
        self.data.lock().unwrap().split_strand(&nucl, None);
    }

    pub fn split_strand_forced_end(&self, nucl: Nucl, forced_end: Option<bool>) {
        self.data.lock().unwrap().split_strand(&nucl, forced_end);
    }

    pub fn rm_helix(&self, helix: usize) {
        self.data.lock().unwrap().remove_helix(helix)
    }

    pub fn get_grid_instance(&self) -> Vec<GridInstance> {
        self.data.lock().unwrap().get_grid_instances(self.id)
    }

    pub fn get_grid2d(&self, id: usize) -> Option<Arc<RwLock<Grid2D>>> {
        self.data.lock().unwrap().get_grid(id)
    }

    pub fn get_grid_basis(&self, g_id: usize) -> Option<ultraviolet::Rotor3> {
        self.data.lock().unwrap().get_grid_basis(g_id)
    }

    pub fn get_helices_grid(&self, g_id: usize) -> Option<HashSet<usize>> {
        self.data.lock().unwrap().get_helices_grid(g_id)
    }

    pub fn get_helices_grid_coord(&self, g_id: usize) -> Option<Vec<(isize, isize)>> {
        self.data.lock().unwrap().get_helices_grid_coord(g_id)
    }

    pub fn get_helices_grid_key_coord(&self, g_id: usize) -> Option<Vec<((isize, isize), usize)>> {
        self.data.lock().unwrap().get_helices_grid_key_coord(g_id)
    }

    pub fn get_helix_grid(&self, g_id: usize, x: isize, y: isize) -> Option<u32> {
        self.data.lock().unwrap().get_helix_grid(g_id, x, y)
    }

    pub fn get_grid_position(&self, g_id: usize) -> Option<ultraviolet::Vec3> {
        self.data.lock().unwrap().get_grid_position(g_id)
    }

    pub fn get_grid_latice_position(
        &self,
        g_id: usize,
        x: isize,
        y: isize,
    ) -> Option<ultraviolet::Vec3> {
        self.data
            .lock()
            .unwrap()
            .get_grid_latice_position(g_id, x, y)
    }

    pub fn get_grid_pos_helix(&self, h_id: u32) -> Option<GridPosition> {
        self.data.lock().unwrap().get_grid_pos_helix(h_id)
    }

    pub fn build_helix_grid(
        &mut self,
        g_id: usize,
        x: isize,
        y: isize,
        position: isize,
        length: usize,
    ) {
        self.data
            .lock()
            .unwrap()
            .build_helix_grid(g_id, x, y, position, length)
    }

    pub fn get_persistent_phantom_helices(&self) -> HashSet<u32> {
        self.data.lock().unwrap().get_persistent_phantom_helices()
    }

    pub fn get_nucl(&self, e_id: u32) -> Option<Nucl> {
        self.data.lock().unwrap().get_nucl(e_id)
    }

    pub fn get_nucl_relax(&self, e_id: u32) -> Option<Nucl> {
        let data = self.data.lock().unwrap();
        data.get_nucl(e_id).or(data.get_bound_5prime(e_id))
    }

    pub fn has_persistent_phantom(&self, g_id: &usize) -> bool {
        self.data.lock().unwrap().has_persistent_phantom(g_id)
    }

    pub fn set_persistent_phantom(&self, g_id: &usize, persistent: bool) {
        self.data
            .lock()
            .unwrap()
            .set_persistent_phantom(g_id, persistent);
    }

    pub fn set_small_spheres(&self, g_id: &usize, small: bool) {
        println!("setting small {} {}", *g_id, small);
        self.data.lock().unwrap().set_small_spheres(g_id, small);
    }

    pub fn has_small_spheres_nucl_id(&self, n_id: u32) -> bool {
        let helix = self.get_nucl(n_id).map(|n| n.helix);
        helix
            .as_ref()
            .map(|h_id| self.helix_has_small_spheres(h_id))
            .unwrap_or(false)
    }

    pub fn helix_has_small_spheres(&self, h_id: &usize) -> bool {
        self.data.lock().unwrap().helix_has_small_spheres(h_id)
    }

    pub fn has_small_spheres(&self, g_id: &usize) -> bool {
        self.data.lock().unwrap().has_small_spheres(g_id)
    }

    pub fn helix_is_empty(&self, helix: usize) -> bool {
        self.data.lock().unwrap().helix_is_empty(helix)
    }

    pub fn get_isometry(&self, h_id: usize) -> Option<ultraviolet::Isometry2> {
        self.data.lock().unwrap().get_isometry_2d(h_id)
    }

    pub fn set_isometry(&self, h_id: usize, isometry: ultraviolet::Isometry2) {
        self.data.lock().unwrap().set_isometry_2d(h_id, isometry)
    }

    pub fn is_xover_end(&self, nucl: &Nucl) -> Extremity {
        self.data.lock().unwrap().is_xover_end(nucl)
    }

    pub fn is_strand_end(&self, nucl: &Nucl) -> Extremity {
        self.data.lock().unwrap().is_strand_end(nucl)
    }

    pub fn get_strand_nucl(&self, nucl: &Nucl) -> Option<usize> {
        self.data.lock().unwrap().get_strand_nucl(nucl)
    }

    pub fn has_helix(&self, h_id: usize) -> bool {
        self.data.lock().unwrap().has_helix(h_id)
    }

    pub fn view_need_reset(&self) -> bool {
        self.data.lock().unwrap().view_need_reset()
    }

    pub fn get_raw_helix(&self, h_id: usize) -> Option<Helix> {
        self.data.lock().unwrap().get_helix(h_id)
    }

    pub fn get_raw_strand(&self, s_id: usize) -> Option<Strand> {
        self.data.lock().unwrap().get_strand(s_id)
    }

    pub fn get_basis_map(&self) -> Arc<RwLock<HashMap<Nucl, char, RandomState>>> {
        self.data.lock().unwrap().get_basis_map()
    }

    pub fn is_scaffold(&self, s_id: usize) -> bool {
        self.data.lock().unwrap().is_scaffold(s_id)
    }

    pub fn set_scaffold_id(&mut self, scaffold_id: Option<usize>) {
        self.data.lock().unwrap().set_scaffold_id(scaffold_id)
    }

    pub fn set_scaffold_sequence(&mut self, sequence: String, shift: usize) {
        self.data
            .lock()
            .unwrap()
            .set_scaffold_sequence(sequence, shift)
    }

    pub fn set_scaffold_shift(&mut self, shift: usize) {
        self.data.lock().unwrap().set_scaffold_shift(shift)
    }

    pub fn scaffold_is_set(&self) -> bool {
        self.data.lock().unwrap().scaffold_is_set()
    }

    pub fn scaffold_sequence_set(&self) -> bool {
        self.data.lock().unwrap().scaffold_sequence_set()
    }

    pub fn get_stapple_mismatch(&self) -> Option<Nucl> {
        self.data.lock().unwrap().get_stapple_mismatch()
    }

    pub fn get_scaffold_sequence_len(&self) -> Option<usize> {
        self.data.lock().unwrap().get_scaffold_sequence_len()
    }

    pub fn get_scaffold_len(&self) -> Option<usize> {
        self.data.lock().unwrap().get_scaffold_len()
    }

    pub fn get_stapples(&self) -> Vec<Stapple> {
        self.data.lock().unwrap().get_stapples()
    }

    pub fn optimize_shift(&self, channel: std::sync::mpsc::Sender<f32>) -> (usize, String) {
        self.data.lock().unwrap().optimize_shift(channel)
    }

    /// Return the map whose keys are the id of strands that are in a group and the values are the
    /// corresponding group.
    pub fn get_groups(&self) -> Arc<RwLock<BTreeMap<usize, bool>>> {
        self.data.lock().unwrap().get_groups()
    }

    /// Change the group to which a strand belong
    pub fn flip_group(&mut self, h_id: usize) {
        self.data.lock().unwrap().flip_group(h_id)
    }

    pub fn get_suggestions(&self) -> Vec<(Nucl, Nucl)> {
        self.data.lock().unwrap().get_suggestions()
    }

    /// Return a string describing the decomposition of the length of the strand `s_id` into the
    /// sum of the length of its domains
    pub fn decompose_length(&self, s_id: usize) -> String {
        self.data.lock().unwrap().decompose_length(s_id)
    }

    /// Change the color of all the strands in the design, except the scaffold.
    pub fn recolor_stapples(&mut self) {
        self.data.lock().unwrap().recolor_stapples();
    }

    pub fn oxdna_export(&self) {
        self.data.lock().unwrap().oxdna_export();
    }

    /// Merge all the consecutives domains in the design
    pub fn clean_up_domains(&mut self) {
        self.data.lock().unwrap().clean_up_domains()
    }

    /// Start or stop a physicall simulation
    pub fn roll_request(&mut self, request: SimulationRequest, computing: Arc<Mutex<bool>>) {
        self.data.lock().unwrap().roll_request(request, computing);
    }

    pub fn get_xover_info(&self, source: Nucl, target: Nucl) -> Option<XoverInfo> {
        self.data
            .lock()
            .unwrap()
            .get_xover_info(source, target, self.id)
    }

    /// Get the torsion map of the design.
    /// See `Data::get_torsions`
    pub fn get_torsions(&self) -> HashMap<(Nucl, Nucl), Torsion> {
        self.data.lock().unwrap().get_torsions()
    }

    pub fn notify_death(&self) {
        self.data.lock().unwrap().notify_death()
    }

    pub fn get_simulation_state(&self) -> SimulationState {
        self.data.lock().unwrap().get_simulation_state()
    }

    pub fn update_hyperboloid(
        &mut self,
        nb_helix: usize,
        shift: f32,
        length: f32,
        radius_shift: f32,
    ) {
        self.data
            .lock()
            .unwrap()
            .update_hyperboloid(nb_helix, shift, length, radius_shift)
    }

    pub fn roll_helix(&mut self, h_id: usize, roll: f32) {
        self.data.lock().unwrap().roll_helix(h_id, roll)
    }

    pub fn get_roll_helix(&self, h_id: usize) -> Option<f32> {
        self.data.lock().unwrap().get_roll_helix(h_id)
    }

    pub fn request_copy(&mut self, nucl: Nucl) {
        if let Some(s_id) = self.get_strand_nucl(&nucl) {
            self.data.lock().unwrap().set_templates(vec![s_id])
        }
    }

    pub fn request_copy_strands(&mut self, s_ids: Vec<usize>) {
        self.data.lock().unwrap().set_templates(s_ids)
    }

    pub fn request_copy_xovers(&mut self, xover_ids: Vec<usize>) -> bool {
        self.data.lock().unwrap().copy_xovers(xover_ids)
    }

    pub fn request_paste_candidate(&mut self, nucl: Option<Nucl>) {
        self.data.lock().unwrap().set_copy(nucl)
    }

    pub fn request_paste_candidate_xover(&mut self, nucl: Option<Nucl>) {
        self.data.lock().unwrap().paste_xovers(nucl, false);
    }

    pub fn paste(&mut self, nucl: Nucl) -> Option<(StrandState, StrandState)> {
        self.data.lock().unwrap().set_copy(Some(nucl));
        self.data.lock().unwrap().apply_copy()
    }

    pub fn paste_xover(&mut self, nucl: Nucl) -> Option<(StrandState, StrandState)> {
        self.data.lock().unwrap().paste_xovers(Some(nucl), false);
        self.data.lock().unwrap().apply_copy_xovers()
    }

    pub fn apply_duplication(&mut self) -> Option<(StrandState, StrandState)> {
        self.data.lock().unwrap().apply_duplication()
    }

    pub fn apply_duplication_xover(&mut self) -> Option<(StrandState, StrandState)> {
        self.data.lock().unwrap().duplicate_xovers()
    }

    pub fn has_template(&self) -> bool {
        self.data.lock().unwrap().has_template()
    }

    pub fn has_xovers_copy(&self) -> bool {
        self.data.lock().unwrap().has_xovers_copy()
    }

    pub fn get_pasted_position(&self) -> Vec<(Vec<Vec3>, bool)> {
        self.data.lock().unwrap().get_pasted_positions()
    }

    pub fn finalize_hyperboloid(&mut self) {
        self.data.lock().unwrap().finalize_hyperboloid()
    }

    pub fn cancel_hyperboloid(&mut self) {
        self.data.lock().unwrap().clear_hyperboloid()
    }

    pub fn get_xovers_list(&self) -> Vec<(usize, (Nucl, Nucl))> {
        self.data.lock().unwrap().get_xovers_list()
    }

    #[must_use]
    pub fn grid_simulation(
        &mut self,
        time_span: (f32, f32),
        computing: Arc<Mutex<bool>>,
        parameters: RigidBodyConstants,
    ) -> Option<GridSystemState> {
        self.data
            .lock()
            .unwrap()
            .rigid_body_request(time_span, computing, parameters)
    }

    #[must_use]
    pub fn rigid_helices_simulation(
        &mut self,
        time_span: (f32, f32),
        computing: Arc<Mutex<bool>>,
        parameters: RigidBodyConstants,
    ) -> Option<RigidHelixState> {
        self.data
            .lock()
            .unwrap()
            .helix_simulation_request(time_span, computing, parameters)
    }

    pub fn rigid_body_parameters_update(&mut self, parameters: RigidBodyConstants) {
        self.data
            .lock()
            .unwrap()
            .rigid_parameters_update(parameters);
    }

    pub fn get_insertions(&self, s_id: usize) -> Option<Vec<Nucl>> {
        self.data.lock().unwrap().get_insertions(s_id)
    }

    pub fn add_anchor(&mut self, nucl: Nucl) {
        self.data.lock().unwrap().add_anchor(nucl);
    }

    pub fn is_anchor(&self, nucl: Nucl) -> bool {
        self.data.lock().unwrap().is_anchor(nucl)
    }

    pub fn shake_nucl(&self, nucl: Nucl) {
        self.data.lock().unwrap().shake_nucl(nucl)
    }

    pub fn set_new_shift(&mut self, g_id: usize, shift: f32) {
        self.data.lock().unwrap().set_new_shift(g_id, shift)
    }

    pub fn get_shift(&self, g_id: usize) -> Option<f32> {
        self.data.lock().unwrap().get_shift(g_id)
    }

    pub fn get_new_elements(&self) -> Option<Vec<DnaElement>> {
        self.data.lock().unwrap().get_new_elements()
    }

    pub fn update_attribute(&mut self, attribute: DnaAttribute, elements: Vec<DnaElementKey>) {
        let mut data = self.data.lock().unwrap();
        for elt in elements.iter() {
            match attribute {
                DnaAttribute::Visible(b) => match elt {
                    DnaElementKey::Helix(h) => data.set_visibility_helix(*h, b),
                    DnaElementKey::Grid(g) => data.set_visibility_grid(*g, b),
                    _ => (),
                },
                DnaAttribute::XoverGroup(g) => match elt {
                    DnaElementKey::Helix(h) => data.set_group(*h, g),
                    _ => (),
                },
            }
        }
    }

    pub fn update_organizer_tree(&mut self, tree: OrganizerTree<DnaElementKey>) {
        self.data.lock().unwrap().update_organizer_tree(tree)
    }

    pub fn get_organizer_tree(&self) -> Option<OrganizerTree<DnaElementKey>> {
        self.data.lock().unwrap().get_organizer_tree()
    }

    pub fn clear_visibility_sive(&mut self) {
        self.data.lock().unwrap().clear_visibility_sive()
    }

    pub fn set_visibility_sieve(&mut self, selection: Vec<Selection>, compl: bool) {
        self.data
            .lock()
            .unwrap()
            .set_visibility_sieve(selection, compl)
    }

    pub fn get_xover_id(&self, xover: &(Nucl, Nucl)) -> Option<usize> {
        self.data.lock().unwrap().get_xover_id(xover)
    }

    pub fn get_xover_with_id(&self, id: usize) -> Option<(Nucl, Nucl)> {
        self.data.lock().unwrap().get_xover_with_id(id)
    }

    pub fn delete_selection(
        &mut self,
        selection: Vec<Selection>,
    ) -> Option<(StrandState, StrandState)> {
        let init = self.data.lock().unwrap().get_strand_state();
        if self.data.lock().unwrap().delete_selection(selection) {
            let after = self.data.lock().unwrap().get_strand_state();
            Some((init, after))
        } else {
            None
        }
    }

    pub fn get_scaffold_info(&self) -> Option<ScaffoldInfo> {
        self.data.lock().unwrap().get_scaffold_info()
    }

    pub fn has_at_least_on_strand_with_insertions(&self) -> bool {
        self.data
            .lock()
            .unwrap()
            .has_at_least_on_strand_with_insertions()
    }

    pub fn replace_insertions_by_helices(&mut self) {
        self.data.lock().unwrap().replace_all_insertions()
    }

    pub fn get_dna_parameters(&self) -> Parameters {
        self.data.lock().unwrap().get_dna_parameters()
    }

    pub fn get_prime3_set(&self) -> Vec<(Vec3, Vec3, u32)> {
        self.data.lock().unwrap().get_prime3_set()
    }
}

#[derive(Clone)]
pub struct DesignNotification {
    pub design_id: usize,
    pub content: DesignNotificationContent,
}

/// A modification to the design that must be notified to the applications
#[derive(Clone)]
pub enum DesignNotificationContent {
    /// The model matrix of the design has been modified
    ModelChanged(Mat4),
    /// The design was modified
    InstanceChanged,
    ViewNeedReset,
}

/// The referential in which one wants to get an element's coordinates
#[derive(Debug, Clone, Copy)]
pub enum Referential {
    World,
    Model,
}

impl Referential {
    pub fn is_world(&self) -> bool {
        match self {
            Referential::World => true,
            _ => false,
        }
    }
}

/// A stucture that defines an helix on a grid
#[derive(Clone, Debug)]
pub struct GridHelixDescriptor {
    pub grid_id: usize,
    pub x: isize,
    pub y: isize,
}

#[derive(Clone, Debug)]
pub enum OperationResult {
    BigChange(StrandState, StrandState),
    UndoableChange,
    NoChange,
}

#[derive(Clone, Debug)]
pub struct ScaffoldInfo {
    pub id: usize,
    pub shift: Option<usize>,
    pub length: usize,
    pub starting_nucl: Option<Nucl>,
}
