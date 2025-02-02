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
use super::super::maths_3d::{Basis3D, UnalignedBoundaries};
use super::super::view::{
    ConeInstance, Instanciable, RawDnaInstance, SphereInstance, TubeInstance,
};
use super::super::GridInstance;
use super::{LetterInstance, SceneElement, StrandBuilder};
use crate::consts::*;
use crate::design::{Design, Nucl, ObjectType, Referential};
use crate::utils;
use crate::utils::instance::Instance;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::{Arc, RwLock};
use ultraviolet::{Mat4, Rotor3, Vec3};

/// An object that handles the 3d graphcial representation of a `Design`
pub struct Design3D {
    design: Arc<RwLock<Design>>,
    id: u32,
    symbol_map: HashMap<char, usize>,
}

impl Design3D {
    pub fn new(design: Arc<RwLock<Design>>) -> Self {
        let id = design.read().unwrap().get_id() as u32;
        let mut symbol_map = HashMap::new();
        for (s_id, s) in BASIS_SYMBOLS.iter().enumerate() {
            symbol_map.insert(*s, s_id);
        }
        Self {
            design,
            id,
            symbol_map,
        }
    }

    /*
    /// Convert a list of ids into a list of instances
    pub fn id_to_instances(&self, ids: Vec<u32>) -> Vec<Instance> {
        let mut ret = Vec::new();
        for id in ids.iter() {
            if let Some(instance) = self.make_instance(*id) {
                ret.push(instance)
            }
        }
        ret
    }*/

    /// Convert a list of ids into a list of instances
    pub fn id_to_raw_instances(&self, ids: Vec<u32>) -> Vec<RawDnaInstance> {
        let mut ret = Vec::new();
        for id in ids.iter() {
            if let Some(instance) = self.make_raw_instance(*id) {
                ret.push(instance)
            }
        }
        ret
    }

    /*
    /// Return the list of sphere instances to be displayed to represent the design
    pub fn get_spheres(&self) -> Rc<Vec<Instance>> {
        let ids = self.design.lock().unwrap().get_all_nucl_ids();
        Rc::new(self.id_to_instances(ids))
    }
    */

    /// Return the list of raw sphere instances to be displayed to represent the design
    pub fn get_spheres_raw(&self) -> Rc<Vec<RawDnaInstance>> {
        let ids = self.design.read().unwrap().get_all_visible_nucl_ids();
        Rc::new(self.id_to_raw_instances(ids))
    }

    pub fn get_pasted_strand(&self) -> (Vec<RawDnaInstance>, Vec<RawDnaInstance>) {
        let mut spheres = Vec::new();
        let mut tubes = Vec::new();
        let positions = self.design.read().unwrap().get_pasted_position();
        for (positions, pastable) in positions {
            let mut previous_postion = None;
            let color = if pastable {
                CANDIDATE_COLOR
            } else {
                SELECTED_COLOR
            };
            let color_vec4 = Instance::color_from_au32(color);
            for position in positions.iter() {
                let sphere = SphereInstance {
                    position: *position,
                    color: color_vec4,
                    id: 0,
                    radius: 1.,
                }
                .to_raw_instance();
                spheres.push(sphere);
                if let Some(prev) = previous_postion {
                    let tube = create_dna_bound(prev, *position, color, 0, true);
                    tubes.push(tube.to_raw_instance());
                }
                previous_postion = Some(*position);
            }
        }
        (spheres, tubes)
    }

    pub fn get_letter_instances(&self) -> Vec<Vec<LetterInstance>> {
        let ids = self.design.read().unwrap().get_all_nucl_ids();
        let mut vecs = vec![Vec::new(); NB_BASIS_SYMBOLS];
        for id in ids {
            let pos = self.design.read().unwrap().get_symbol_position(id);
            let symbol = self.design.read().unwrap().get_symbol(id);
            if let Some((pos, symbol)) = pos.zip(symbol) {
                if let Some(id) = self.symbol_map.get(&symbol) {
                    let instance = LetterInstance {
                        position: pos,
                        color: ultraviolet::Vec4::new(0., 0., 0., 1.),
                        design_id: self.id,
                        scale: 1.,
                        shift: Vec3::zero(),
                    };
                    vecs[*id].push(instance);
                }
            }
        }
        vecs
    }

    /*
    /// Return the list of tube instances to be displayed to represent the design
    pub fn get_tubes(&self) -> Rc<Vec<Instance>> {
        let ids = self.design.read().unwrap().get_all_bound_ids();
        Rc::new(self.id_to_instances(ids))
    }
    */

    /// Return the list of tube instances to be displayed to represent the design
    pub fn get_tubes_raw(&self) -> Rc<Vec<RawDnaInstance>> {
        let ids = self.design.read().unwrap().get_all_visible_bound_ids();
        Rc::new(self.id_to_raw_instances(ids))
    }

    pub fn get_model_matrix(&self) -> Mat4 {
        self.design.read().unwrap().get_model_matrix()
    }

    /// Convert return an instance representing the object with identifier `id` and custom
    /// color and radius.
    pub fn make_instance(&self, id: u32, color: u32, mut radius: f32) -> Option<RawDnaInstance> {
        let kind = self.get_object_type(id)?;
        let referential = Referential::Model;
        let instanciable = match kind {
            ObjectType::Bound(id1, id2) => {
                let pos1 = self.get_design_element_position(id1, referential)?;
                let pos2 = self.get_design_element_position(id2, referential)?;
                let id = id | self.id << 24;
                create_dna_bound(pos1, pos2, color, id, true)
                    .with_radius(radius)
                    .to_raw_instance()
            }
            ObjectType::Nucleotide(id) => {
                let position = self.get_design_element_position(id, referential)?;
                let id = id | self.id << 24;
                let color = Instance::color_from_au32(color);
                let small = self.design.read().unwrap().has_small_spheres_nucl_id(id);
                if radius > 1.01 && small {
                    radius *= 2.5;
                }
                radius = if small { radius / 3.5 } else { radius };
                SphereInstance {
                    position,
                    radius,
                    color,
                    id,
                }
                .to_raw_instance()
            }
        };
        Some(instanciable)
    }

    /// Convert return an instance representing the object with identifier `id`
    pub fn make_raw_instance(&self, id: u32) -> Option<RawDnaInstance> {
        let kind = self.get_object_type(id)?;
        let referential = Referential::Model;
        let raw_instance = match kind {
            ObjectType::Bound(id1, id2) => {
                let pos1 = self.get_design_element_position(id1, referential)?;
                let pos2 = self.get_design_element_position(id2, referential)?;
                let color = self.get_color(id).unwrap_or(0);
                let id = id | self.id << 24;
                let tube = create_dna_bound(pos1, pos2, color, id, false);
                tube.to_raw_instance()
            }
            ObjectType::Nucleotide(id) => {
                let position = self.get_design_element_position(id, referential)?;
                let color = self.get_color(id)?;
                let color = Instance::color_from_u32(color);
                let id = id | self.id << 24;
                let small = self.design.read().unwrap().has_small_spheres_nucl_id(id);
                let radius = if small {
                    BOUND_RADIUS / SPHERE_RADIUS
                } else {
                    1.
                };
                let sphere = SphereInstance {
                    position,
                    color,
                    id,
                    radius,
                };
                sphere.to_raw_instance()
            }
        };
        Some(raw_instance)
    }

    pub fn get_suggested_spheres(&self) -> Vec<RawDnaInstance> {
        let suggestion = self.design.read().unwrap().get_suggestions();
        let mut ret = vec![];
        for (n1, n2) in suggestion {
            let nucl_1 = self
                .design
                .read()
                .unwrap()
                .get_helix_nucl(n1, Referential::Model, false);
            let nucl_2 = self
                .design
                .read()
                .unwrap()
                .get_helix_nucl(n2, Referential::Model, false);
            if let Some(position) = nucl_1 {
                let instance = SphereInstance {
                    color: Instance::color_from_au32(SUGGESTION_COLOR),
                    position,
                    id: 0,
                    radius: SELECT_SCALE_FACTOR,
                }
                .to_raw_instance();
                ret.push(instance);
            }
            if let Some(position) = nucl_2 {
                let instance = SphereInstance {
                    color: Instance::color_from_au32(SUGGESTION_COLOR),
                    position,
                    id: 0,
                    radius: SELECT_SCALE_FACTOR,
                }
                .to_raw_instance();
                ret.push(instance);
            }
        }
        ret
    }

    pub fn get_suggested_tubes(&self) -> Vec<RawDnaInstance> {
        let suggestion = self.design.read().unwrap().get_suggestions();
        let mut ret = vec![];
        for (n1, n2) in suggestion {
            let nucl_1 = self
                .design
                .read()
                .unwrap()
                .get_helix_nucl(n1, Referential::Model, false);
            let nucl_2 = self
                .design
                .read()
                .unwrap()
                .get_helix_nucl(n2, Referential::Model, false);
            if let Some((position1, position2)) = nucl_1.zip(nucl_2) {
                let instance = create_dna_bound(position1, position2, SUGGESTION_COLOR, 0, true)
                    .to_raw_instance();
                ret.push(instance);
            }
        }
        ret
    }

    /// Make a instance with the same postion and orientation as a phantom element.
    pub fn make_instance_phantom(
        &self,
        phantom_element: &utils::PhantomElement,
        color: u32,
        radius: f32,
    ) -> Option<RawDnaInstance> {
        let nucl = Nucl {
            helix: phantom_element.helix_id as usize,
            position: phantom_element.position as isize,
            forward: phantom_element.forward,
        };
        let helix_id = phantom_element.helix_id;
        let i = phantom_element.position;
        let forward = phantom_element.forward;
        if phantom_element.bound {
            let nucl_1 =
                self.design
                    .read()
                    .unwrap()
                    .get_helix_nucl(nucl, Referential::Model, false)?;
            let nucl_2 = self.design.read().unwrap().get_helix_nucl(
                nucl.left(),
                Referential::Model,
                false,
            )?;
            let id = utils::phantom_helix_encoder_bound(self.id, helix_id, i, forward);
            Some(create_dna_bound(nucl_1, nucl_2, color, id, true).to_raw_instance())
        } else {
            let nucl_coord =
                self.design
                    .read()
                    .unwrap()
                    .get_helix_nucl(nucl, Referential::Model, false)?;
            let id = utils::phantom_helix_encoder_nucl(self.id, helix_id, i, forward);
            let instance = SphereInstance {
                color: Instance::color_from_au32(color),
                position: nucl_coord,
                id,
                radius,
            }
            .to_raw_instance();
            Some(instance)
        }
    }

    pub fn get_phantom_element_position(
        &self,
        phantom_element: &utils::PhantomElement,
        referential: Referential,
        on_axis: bool,
    ) -> Option<Vec3> {
        let helix_id = phantom_element.helix_id;
        let i = phantom_element.position;
        let forward = phantom_element.forward;
        let nucl = Nucl {
            helix: helix_id as usize,
            position: i as isize,
            forward,
        };
        if phantom_element.bound {
            let nucl_1 = self
                .design
                .read()
                .unwrap()
                .get_helix_nucl(nucl, referential, on_axis)?;
            let nucl_2 =
                self.design
                    .read()
                    .unwrap()
                    .get_helix_nucl(nucl.left(), referential, on_axis)?;
            Some((nucl_1 + nucl_2) / 2.)
        } else {
            let nucl_coord = self
                .design
                .read()
                .unwrap()
                .get_helix_nucl(nucl, referential, on_axis);
            nucl_coord
        }
    }

    pub fn make_phantom_helix_instances_raw(
        &self,
        helix_ids: &HashMap<u32, bool>,
    ) -> (Rc<Vec<RawDnaInstance>>, Rc<Vec<RawDnaInstance>>) {
        let mut spheres = Vec::new();
        let mut tubes = Vec::new();
        for (helix_id, short) in helix_ids.iter() {
            let range_phantom = if *short {
                PHANTOM_RANGE / 10
            } else {
                PHANTOM_RANGE
            };
            for forward in [false, true].iter() {
                let mut previous_nucl = None;
                for i in -range_phantom..=range_phantom {
                    let nucl_coord = self.design.read().unwrap().get_helix_nucl(
                        Nucl {
                            helix: *helix_id as usize,
                            position: i as isize,
                            forward: *forward,
                        },
                        Referential::Model,
                        false,
                    );
                    let color = 0xFFD0D0D0;
                    if nucl_coord.is_none() {
                        continue;
                    }
                    let nucl_coord = nucl_coord.unwrap();
                    let id = utils::phantom_helix_encoder_nucl(self.id, *helix_id, i, *forward);
                    spheres.push(
                        SphereInstance {
                            position: nucl_coord,
                            color: Instance::color_from_au32(color),
                            id,
                            radius: 0.6,
                        }
                        .to_raw_instance(),
                    );
                    if let Some(coord) = previous_nucl {
                        let id =
                            utils::phantom_helix_encoder_bound(self.id, *helix_id, i, *forward);
                        tubes.push(
                            create_dna_bound(nucl_coord, coord, color, id, true)
                                .with_radius(0.6)
                                .to_raw_instance(),
                        );
                    }
                    previous_nucl = Some(nucl_coord);
                }
            }
        }
        (Rc::new(spheres), Rc::new(tubes))
    }

    fn get_object_type(&self, id: u32) -> Option<ObjectType> {
        self.design.read().unwrap().get_object_type(id)
    }

    pub fn get_bound(&self, id: u32) -> Option<(Nucl, Nucl)> {
        if let Some(ObjectType::Bound(n1, n2)) = self.get_object_type(id) {
            self.get_nucl(n1).zip(self.get_nucl(n2))
        } else {
            None
        }
    }

    pub fn get_element_position(
        &self,
        element: &SceneElement,
        referential: Referential,
    ) -> Option<Vec3> {
        match element {
            SceneElement::DesignElement(_, e_id) => {
                self.get_design_element_position(*e_id, referential)
            }
            SceneElement::PhantomElement(phantom) => {
                self.get_phantom_element_position(phantom, referential, false)
            }
            SceneElement::Grid(_, g_id) => self.design.read().unwrap().get_grid_position(*g_id),
            SceneElement::GridCircle(_, g_id, x, y) => self
                .design
                .read()
                .unwrap()
                .get_grid_latice_position(*g_id, *x, *y),
            _ => None,
        }
    }

    pub fn get_element_axis_position(
        &self,
        element: &SceneElement,
        referential: Referential,
    ) -> Option<Vec3> {
        match element {
            SceneElement::DesignElement(_, e_id) => {
                self.get_design_element_axis_position(*e_id, referential)
            }
            SceneElement::PhantomElement(phantom) => {
                self.get_phantom_element_position(phantom, referential, true)
            }
            SceneElement::WidgetElement(_)
            | SceneElement::Grid(_, _)
            | SceneElement::GridCircle(_, _, _, _) => None,
        }
    }

    pub fn get_design_element_position(&self, id: u32, referential: Referential) -> Option<Vec3> {
        self.design
            .read()
            .unwrap()
            .get_element_position(id, referential)
    }

    pub fn get_design_element_axis_position(
        &self,
        id: u32,
        referential: Referential,
    ) -> Option<Vec3> {
        self.design
            .read()
            .unwrap()
            .get_element_axis_position(id, referential)
    }

    fn get_color(&self, id: u32) -> Option<u32> {
        self.design.read().unwrap().get_color(id)
    }

    /// Return the middle point of `self` in the world coordinates
    pub fn middle_point(&self) -> Vec3 {
        let boundaries = self.boundaries();
        let middle = Vec3::new(
            (boundaries[0] + boundaries[1]) as f32 / 2.,
            (boundaries[2] + boundaries[3]) as f32 / 2.,
            (boundaries[4] + boundaries[5]) as f32 / 2.,
        );
        self.design
            .read()
            .unwrap()
            .get_model_matrix()
            .transform_vec3(middle)
    }

    fn boundaries(&self) -> [f32; 6] {
        let mut min_x = std::f32::INFINITY;
        let mut min_y = std::f32::INFINITY;
        let mut min_z = std::f32::INFINITY;
        let mut max_x = std::f32::NEG_INFINITY;
        let mut max_y = std::f32::NEG_INFINITY;
        let mut max_z = std::f32::NEG_INFINITY;

        let ids = self.design.read().unwrap().get_all_nucl_ids();
        for id in ids {
            let coord: [f32; 3] = self
                .design
                .read()
                .unwrap()
                .get_element_position(id, Referential::World)
                .unwrap()
                .into();
            if coord[0] < min_x {
                min_x = coord[0];
            }
            if coord[0] > max_x {
                max_x = coord[0];
            }
            if coord[1] < min_y {
                min_y = coord[1];
            }
            if coord[1] > max_y {
                max_y = coord[1];
            }
            if coord[2] < min_z {
                min_z = coord[2];
            }
            if coord[2] > max_z {
                max_z = coord[2];
            }
        }
        for grid in self.get_grid().iter() {
            let coords: [[f32; 3]; 2] = [
                grid.grid
                    .position_helix(grid.min_x as isize, grid.min_y as isize)
                    .into(),
                grid.grid
                    .position_helix(grid.max_x as isize, grid.max_y as isize)
                    .into(),
            ];
            for coord in coords.iter() {
                if coord[0] < min_x {
                    min_x = coord[0];
                }
                if coord[0] > max_x {
                    max_x = coord[0];
                }
                if coord[1] < min_y {
                    min_y = coord[1];
                }
                if coord[1] > max_y {
                    max_y = coord[1];
                }
                if coord[2] < min_z {
                    min_z = coord[2];
                }
                if coord[2] > max_z {
                    max_z = coord[2];
                }
            }
        }
        [min_x, max_x, min_y, max_y, min_z, max_z]
    }

    fn get_all_grid_corners(&self) -> Vec<Vec3> {
        let mut ret = Vec::new();
        for grid in self.get_grid().iter() {
            ret.push(
                grid.grid
                    .position_helix(grid.min_x as isize, grid.min_y as isize),
            );
            ret.push(
                grid.grid
                    .position_helix(grid.min_x as isize, grid.max_y as isize),
            );
            ret.push(
                grid.grid
                    .position_helix(grid.max_x as isize, grid.min_y as isize),
            );
            ret.push(
                grid.grid
                    .position_helix(grid.max_x as isize, grid.max_y as isize),
            );
        }
        ret
    }

    fn get_all_points(&self) -> Vec<Vec3> {
        let ids = self.design.read().unwrap().get_all_nucl_ids();
        let mut ret: Vec<Vec3> = ids
            .iter()
            .filter_map(|id| {
                self.design
                    .read()
                    .unwrap()
                    .get_element_position(*id, Referential::World)
            })
            .collect();
        ret.extend(self.get_all_grid_corners().into_iter());
        ret
    }

    fn boundaries_unaligned(&self, basis: Basis3D) -> UnalignedBoundaries {
        let mut ret = UnalignedBoundaries::from_basis(basis);
        for point in self.get_all_points().into_iter() {
            ret.add_point(point)
        }
        ret
    }

    pub fn get_fitting_camera_position(
        &self,
        basis: Basis3D,
        fovy: f32,
        ratio: f32,
    ) -> Option<Vec3> {
        let boundaries = self.boundaries_unaligned(basis);
        boundaries.fit_point(fovy, ratio)
    }

    pub fn get_all_elements(&self) -> HashSet<u32> {
        let mut ret = HashSet::new();
        for x in self.design.read().unwrap().get_all_nucl_ids().iter() {
            ret.insert(*x);
        }
        for x in self.design.read().unwrap().get_all_bound_ids().iter() {
            ret.insert(*x);
        }
        ret
    }

    pub fn get_strand(&self, element_id: u32) -> u32 {
        self.design.read().unwrap().get_strand(element_id).unwrap() as u32
    }

    pub fn get_helix(&self, element_id: u32) -> u32 {
        self.design.read().unwrap().get_helix(element_id).unwrap() as u32
    }

    pub fn get_strand_elements(&self, strand_id: u32) -> HashSet<u32> {
        self.design
            .read()
            .unwrap()
            .get_strand_elements(strand_id as usize)
            .into_iter()
            .collect()
    }

    pub fn get_element_type(&self, e_id: u32) -> Option<ObjectType> {
        self.design.read().unwrap().get_object_type(e_id)
    }

    pub fn get_helix_elements(&self, helix_id: u32) -> HashSet<u32> {
        self.design
            .read()
            .unwrap()
            .get_helix_elements(helix_id as usize)
            .into_iter()
            .collect()
    }

    pub fn get_helix_basis(&self, h_id: u32) -> Option<Rotor3> {
        self.design.read().unwrap().get_helix_basis(h_id)
    }

    pub fn get_basis(&self) -> Rotor3 {
        self.design.read().unwrap().get_basis()
    }

    pub fn get_element_5prime(&self, element_id: u32) -> Option<SceneElement> {
        let id = self.design.read().unwrap().get_element_5prime(element_id)?;
        Some(SceneElement::DesignElement(self.id, id))
    }

    pub fn get_element_3prime(&self, element_id: u32) -> Option<SceneElement> {
        let id = self.design.read().unwrap().get_element_3prime(element_id)?;
        Some(SceneElement::DesignElement(self.id, id))
    }

    pub fn get_identifier_nucl(&self, nucl: &Nucl) -> Option<u32> {
        self.design.read().unwrap().get_identifier_nucl(nucl)
    }

    pub fn get_identifier_bound(&self, n1: &Nucl, n2: &Nucl) -> Option<u32> {
        self.design.read().unwrap().get_identifier_bound(n1, n2)
    }

    pub fn get_element_identifier_from_xover_id(&self, xover_id: usize) -> Option<u32> {
        self.design
            .read()
            .unwrap()
            .get_xover_with_id(xover_id)
            .and_then(|(n1, n2)| self.design.read().unwrap().get_identifier_bound(&n1, &n2))
    }

    pub fn get_xover_id(&self, xover: &(Nucl, Nucl)) -> Option<usize> {
        self.design.read().unwrap().get_xover_id(xover)
    }

    pub fn get_xover_with_id(&self, xover_id: usize) -> Option<(Nucl, Nucl)> {
        self.design.read().unwrap().get_xover_with_id(xover_id)
    }

    pub fn get_builder(&self, element: &SceneElement, stick: bool) -> Option<StrandBuilder> {
        match element {
            SceneElement::DesignElement(_, e_id) => self
                .design
                .write()
                .unwrap()
                .get_builder_element(*e_id, stick),
            SceneElement::PhantomElement(phantom_element) => {
                let nucl = Nucl {
                    helix: phantom_element.helix_id as usize,
                    position: phantom_element.position as isize,
                    forward: phantom_element.forward,
                };
                self.design.read().unwrap().get_builder(nucl, stick)
            }
            _ => None,
        }
    }

    pub fn get_grid(&self) -> Vec<GridInstance> {
        self.design.read().unwrap().get_grid_instance()
    }

    pub fn get_helices_grid(&self, g_id: usize) -> Option<HashSet<usize>> {
        self.design.read().unwrap().get_helices_grid(g_id)
    }

    pub fn get_helices_grid_coord(&self, g_id: usize) -> Vec<(isize, isize)> {
        self.design
            .read()
            .unwrap()
            .get_helices_grid_coord(g_id)
            .unwrap_or(Vec::new())
    }

    pub fn get_helices_grid_key_coord(&self, g_id: usize) -> Vec<((isize, isize), usize)> {
        self.design
            .read()
            .unwrap()
            .get_helices_grid_key_coord(g_id)
            .unwrap_or(Vec::new())
    }

    pub fn get_helix_grid(&self, g_id: usize, x: isize, y: isize) -> Option<u32> {
        self.design.read().unwrap().get_helix_grid(g_id, x, y)
    }

    pub fn get_persistent_phantom_helices(&self) -> HashSet<u32> {
        self.design.read().unwrap().get_persistent_phantom_helices()
    }

    pub fn get_grid_basis(&self, g_id: usize) -> Option<Rotor3> {
        self.design.read().unwrap().get_grid_basis(g_id)
    }

    pub fn get_nucl(&self, e_id: u32) -> Option<Nucl> {
        self.design.read().unwrap().get_nucl(e_id)
    }

    pub fn get_nucl_relax(&self, e_id: u32) -> Option<Nucl> {
        self.design.read().unwrap().get_nucl_relax(e_id)
    }

    pub fn helix_is_on_grid(&self, h_id: u32) -> bool {
        self.design
            .read()
            .unwrap()
            .get_grid_pos_helix(h_id)
            .is_some()
    }

    pub fn get_nucl_position(&self, nucl: Nucl) -> Option<Vec3> {
        self.design
            .read()
            .unwrap()
            .get_helix_nucl(nucl, Referential::World, false)
    }

    pub fn pivot_sphere(position: Vec3) -> RawDnaInstance {
        SphereInstance {
            position,
            id: 0,
            radius: 1.2 * SELECT_SCALE_FACTOR,
            color: Instance::color_from_au32(PIVOT_SPHERE_COLOR),
        }
        .to_raw_instance()
    }

    pub fn free_xover_sphere(position: Vec3) -> RawDnaInstance {
        SphereInstance {
            position,
            id: 0,
            radius: 1.1 * SELECT_SCALE_FACTOR,
            color: Instance::color_from_au32(FREE_XOVER_COLOR),
        }
        .to_raw_instance()
    }

    pub fn free_xover_tube(pos1: Vec3, pos2: Vec3) -> RawDnaInstance {
        create_dna_bound(pos1, pos2, FREE_XOVER_COLOR, 0, true).to_raw_instance()
    }

    pub fn has_nucl(&self, nucl: &Nucl) -> bool {
        self.design
            .read()
            .unwrap()
            .get_identifier_nucl(nucl)
            .is_some()
    }

    pub fn both_prime3(&self, nucl1: Nucl, nucl2: Nucl) -> bool {
        let prime3_1 = self.design.read().unwrap().prime3_of(nucl1);
        let prime3_2 = self.design.read().unwrap().prime3_of(nucl2);
        prime3_1.and(prime3_2).is_some()
    }

    pub fn both_prime5(&self, nucl1: Nucl, nucl2: Nucl) -> bool {
        let prime5_1 = self.design.read().unwrap().prime5_of(nucl1);
        let prime5_2 = self.design.read().unwrap().prime5_of(nucl2);
        prime5_1.and(prime5_2).is_some()
    }

    pub fn get_all_prime3_cone(&self) -> Vec<RawDnaInstance> {
        let cones = self.design.read().unwrap().get_prime3_set();
        let mut ret = Vec::with_capacity(cones.len());
        for c in cones {
            ret.push(create_prime3_cone(c.0, c.1, c.2));
        }
        ret
    }
}

fn create_dna_bound(
    source: Vec3,
    dest: Vec3,
    color: u32,
    id: u32,
    use_alpha: bool,
) -> TubeInstance {
    let color = if use_alpha {
        Instance::color_from_au32(color)
    } else {
        Instance::color_from_u32(color)
    };
    let rotor = Rotor3::from_rotation_between(Vec3::unit_x(), (dest - source).normalized());
    let position = (dest + source) / 2.;
    let length = (dest - source).mag();

    TubeInstance {
        position,
        color,
        rotor,
        id,
        radius: 1.,
        length,
    }
}

fn create_prime3_cone(source: Vec3, dest: Vec3, color: u32) -> RawDnaInstance {
    let color = Instance::color_from_u32(color);
    let rotor = Rotor3::from_rotation_between(Vec3::unit_x(), (dest - source).normalized());
    let position = source;
    let length = 2. / 3. * (dest - source).mag();
    ConeInstance {
        position,
        length,
        rotor,
        color,
        id: 0,
        radius: 1.5 * SPHERE_RADIUS,
    }
    .to_raw_instance()
}
