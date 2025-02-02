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
use super::*;
use ahash::RandomState;
use mathru::algebra::linear::vector::vector::Vector;
use mathru::analysis::differential_equation::ordinary::{ExplicitEuler, ExplicitODE, Kutta3};
use ordered_float::OrderedFloat;
use rand::Rng;
use rand_distr::{Exp, StandardNormal};
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use ultraviolet::{Bivec3, Mat3, Rotor3, Vec3};

#[derive(Debug)]
struct HelixSystem {
    springs: Vec<(RigidNucl, RigidNucl)>,
    free_springs: Vec<(usize, usize)>,
    mixed_springs: Vec<(RigidNucl, usize)>,
    free_nucls: Vec<FreeNucl>,
    free_nucl_position: Vec<Vec3>,
    helices: Vec<RigidHelix>,
    time_span: (f32, f32),
    last_state: Option<Vector<f32>>,
    parameters: Parameters,
    anchors: Vec<(RigidNucl, Vec3)>,
    free_anchors: Vec<(usize, Vec3)>,
    current_time: f32,
    next_time: f32,
    brownian_heap: BinaryHeap<(Reverse<OrderedFloat<f32>>, usize)>,
    rigid_parameters: RigidBodyConstants,
    max_time_step: f32,
}

#[derive(Clone, Debug)]
pub struct RigidBodyConstants {
    pub k_spring: f32,
    pub k_friction: f32,
    pub mass: f32,
    pub volume_exclusion: bool,
    pub brownian_motion: bool,
    pub brownian_rate: f32,
    pub brownian_amplitude: f32,
}

#[derive(Debug)]
struct RigidNucl {
    helix: usize,
    position: isize,
    forward: bool,
}

#[derive(Debug, Hash, Eq, PartialEq, Clone, Copy)]
struct FreeNucl {
    helix: Option<usize>,
    position: isize,
    forward: bool,
    old_helix: Option<usize>,
}

impl FreeNucl {
    fn with_helix(nucl: &Nucl, helix: Option<usize>) -> Self {
        Self {
            helix,
            position: nucl.position,
            forward: nucl.forward,
            old_helix: helix.xor(Some(nucl.helix)),
        }
    }
}

impl HelixSystem {
    fn forces_and_torques(
        &self,
        positions: &[Vec3],
        orientations: &[Rotor3],
    ) -> (Vec<Vec3>, Vec<Vec3>) {
        let nb_element = self.helices.len() + self.free_nucls.len();
        let mut forces = vec![Vec3::zero(); nb_element];
        let mut torques = vec![Vec3::zero(); nb_element];

        const L0: f32 = 0.7;
        const C_VOLUME: f32 = 2f32;
        let k_anchor = 1000. * self.rigid_parameters.k_spring;

        let point_conversion = |nucl: &RigidNucl| {
            let position = positions[nucl.helix]
                + self.helices[nucl.helix]
                    .center_to_origin
                    .rotated_by(orientations[nucl.helix]);
            let mut helix = Helix::new(position, orientations[nucl.helix]);
            helix.roll(self.helices[nucl.helix].roll);
            helix.space_pos(&self.parameters, nucl.position, nucl.forward)
        };
        let free_nucl_pos = |n: &usize| positions[*n + self.helices.len()];

        for spring in self.springs.iter() {
            let point_0 = point_conversion(&spring.0);
            let point_1 = point_conversion(&spring.1);
            let len = (point_1 - point_0).mag();
            let norm = len - L0;

            // The force applied on point 0
            let force = if len > 1e-5 {
                self.rigid_parameters.k_spring * norm * (point_1 - point_0) / len
            } else {
                Vec3::zero()
            };

            forces[spring.0.helix] += 10. * force;
            forces[spring.1.helix] -= 10. * force;

            let torque0 = (point_0 - positions[spring.0.helix]).cross(force);
            let torque1 = (point_1 - positions[spring.1.helix]).cross(-force);

            torques[spring.0.helix] += torque0;
            torques[spring.1.helix] += torque1;
        }
        for (nucl, free_nucl_id) in self.mixed_springs.iter() {
            let point_0 = point_conversion(nucl);
            let point_1 = free_nucl_pos(free_nucl_id);
            let len = (point_1 - point_0).mag();
            let norm = len - L0;

            // The force applied on point 0
            let force = if len > 1e-5 {
                self.rigid_parameters.k_spring * norm * (point_1 - point_0) / len
            } else {
                Vec3::zero()
            };
            forces[nucl.helix] += 10. * force;
            forces[self.helices.len() + *free_nucl_id] -= 10. * force;

            let torque0 = (point_0 - positions[nucl.helix]).cross(force);

            torques[nucl.helix] += torque0;
        }
        for (id_0, id_1) in self.free_springs.iter() {
            let point_0 = free_nucl_pos(id_0);
            let point_1 = free_nucl_pos(id_1);
            let len = (point_1 - point_0).mag();
            let norm = len - L0;

            // The force applied on point 0
            let force = if len > 1e-5 {
                self.rigid_parameters.k_spring * norm * (point_1 - point_0) / len
            } else {
                Vec3::zero()
            };
            forces[self.helices.len() + *id_0] += 10. * force;
            forces[self.helices.len() + *id_1] -= 10. * force;
        }

        for (nucl, position) in self.anchors.iter() {
            let point_0 = point_conversion(&nucl);
            let len = (point_0 - *position).mag();
            let force = if len > 1e-5 {
                self.rigid_parameters.k_spring * k_anchor * -(point_0 - *position)
            } else {
                Vec3::zero()
            };

            forces[nucl.helix] += 10. * force;

            let torque0 = (point_0 - positions[nucl.helix]).cross(force);

            torques[nucl.helix] += torque0;
        }
        for (id, position) in self.free_anchors.iter() {
            let point_0 = free_nucl_pos(id);
            let len = (point_0 - *position).mag();
            let force = if len > 1e-5 {
                self.rigid_parameters.k_spring * k_anchor * -(point_0 - *position)
            } else {
                Vec3::zero()
            };

            forces[self.helices.len() + *id] += 10. * force;
        }
        let segments: Vec<(Vec3, Vec3)> = (0..self.helices.len())
            .map(|n| {
                let position =
                    positions[n] + self.helices[n].center_to_origin.rotated_by(orientations[n]);
                let helix = Helix::new(position, orientations[n]);
                (
                    helix.axis_position(&self.parameters, self.helices[n].interval.0),
                    helix.axis_position(&self.parameters, self.helices[n].interval.1),
                )
            })
            .collect();
        if self.rigid_parameters.volume_exclusion {
            for i in 0..self.helices.len() {
                let (a, b) = segments[i];
                for j in (i + 1)..self.helices.len() {
                    let (c, d) = segments[j];
                    let r = 1.;
                    let (dist, vec, point_a, point_c) = distance_segment(a, b, c, d);
                    if dist < 2. * r {
                        // VOLUME EXCLUSION
                        let norm =
                            C_VOLUME * self.rigid_parameters.k_spring * (2. * r - dist).powi(2);
                        forces[i] += norm * vec;
                        forces[j] += -norm * vec;
                        let torque0 = (point_a - positions[i]).cross(norm * vec);
                        let torque1 = (point_c - positions[j]).cross(-norm * vec);
                        torques[i] += torque0;
                        torques[j] += torque1;
                    }
                }
                for nucl_id in 0..self.free_nucls.len() {
                    let point = free_nucl_pos(&nucl_id);
                    let (dist, vec, _, _) = distance_segment(a, b, point, point);
                    let r = 1.35 / 2.;
                    if dist < 2. * r {
                        let norm =
                            C_VOLUME * self.rigid_parameters.k_spring * (2. * r - dist).powi(2);
                        let norm = norm.min(1e4);
                        forces[self.helices.len() + nucl_id] -= norm * vec;
                    }
                }
            }
        }

        (forces, torques)
    }
}

impl HelixSystem {
    fn read_state(&self, x: &Vector<f32>) -> (Vec<Vec3>, Vec<Rotor3>, Vec<Vec3>, Vec<Vec3>) {
        let mut positions = Vec::with_capacity(self.helices.len() + self.free_nucls.len());
        let mut rotations = Vec::with_capacity(self.helices.len() + self.free_nucls.len());
        let mut linear_momentums = Vec::with_capacity(self.helices.len() + self.free_nucls.len());
        let mut angular_momentums = Vec::with_capacity(self.helices.len() + self.free_nucls.len());
        let mut iterator = x.iter();
        let nb_iter = self.helices.len() + self.free_nucls.len();
        for _ in 0..nb_iter {
            let position = Vec3::new(
                *iterator.next().unwrap(),
                *iterator.next().unwrap(),
                *iterator.next().unwrap(),
            );
            let rotation = Rotor3::new(
                *iterator.next().unwrap(),
                Bivec3::new(
                    *iterator.next().unwrap(),
                    *iterator.next().unwrap(),
                    *iterator.next().unwrap(),
                ),
            )
            .normalized();
            let linear_momentum = Vec3::new(
                *iterator.next().unwrap(),
                *iterator.next().unwrap(),
                *iterator.next().unwrap(),
            );
            let angular_momentum = Vec3::new(
                *iterator.next().unwrap(),
                *iterator.next().unwrap(),
                *iterator.next().unwrap(),
            );
            positions.push(position);
            rotations.push(rotation);
            linear_momentums.push(linear_momentum);
            angular_momentums.push(angular_momentum);
        }
        (positions, rotations, linear_momentums, angular_momentums)
    }

    fn next_time(&mut self) {
        self.current_time = self.next_time;
        if let Some((t, _)) = self.brownian_heap.peek() {
            // t.0 because t is a &Reverse<_>
            if self.rigid_parameters.brownian_motion {
                self.next_time = t.0.into_inner().min(self.current_time + self.max_time_step);
            } else {
                self.next_time = self.current_time + self.max_time_step;
            }
        } else {
            self.next_time = self.current_time + self.max_time_step;
        }
        self.time_span = (0., self.next_time - self.current_time);
        println!("max time span {}", self.max_time_step);
        println!("{:?}", self.time_span());
    }

    fn brownian_jump(&mut self) {
        let mut rnd = rand::thread_rng();
        if let Some((t, _)) = self.brownian_heap.peek() {
            // t.0 because t is a &Reverse<_>
            if self.next_time < t.0.into_inner() {
                return;
            }
        }
        if let Some((_, nucl_id)) = self.brownian_heap.pop() {
            let gx: f32 = rnd.sample(StandardNormal);
            let gy: f32 = rnd.sample(StandardNormal);
            let gz: f32 = rnd.sample(StandardNormal);
            if let Some(state) = self.last_state.as_mut() {
                let entry = 13 * (self.helices.len() + nucl_id);
                *state.get_mut(entry) += self.rigid_parameters.brownian_amplitude * gx;
                *state.get_mut(entry + 1) += self.rigid_parameters.brownian_amplitude * gy;
                *state.get_mut(entry + 2) += self.rigid_parameters.brownian_amplitude * gz;
            }

            let exp_law = Exp::new(self.rigid_parameters.brownian_rate).unwrap();
            let new_date = rnd.sample(exp_law) + self.next_time;
            self.brownian_heap.push((Reverse(new_date.into()), nucl_id));
        }
    }

    fn update_parameters(&mut self, parameters: RigidBodyConstants) {
        self.rigid_parameters = parameters;
        self.brownian_heap.clear();
        let mut rnd = rand::thread_rng();
        let exp_law = Exp::new(self.rigid_parameters.brownian_rate).unwrap();
        for i in 0..self.free_nucls.len() {
            if !self.free_anchors.iter().any(|(x, _)| *x == i) {
                let t = rnd.sample(exp_law) + self.next_time;
                self.brownian_heap.push((Reverse(t.into()), i));
            }
        }
    }

    fn shake_nucl(&mut self, nucl: ShakeTarget) {
        let mut rnd = rand::thread_rng();
        let gx: f32 = rnd.sample(StandardNormal);
        let gy: f32 = rnd.sample(StandardNormal);
        let gz: f32 = rnd.sample(StandardNormal);
        let entry = match nucl {
            ShakeTarget::Helix(h_id) => 13 * h_id,
            ShakeTarget::FreeNucl(n) => 13 * (self.helices.len() + n),
        };
        if let Some(state) = self.last_state.as_mut() {
            *state.get_mut(entry) += 10. * self.rigid_parameters.brownian_amplitude * gx;
            *state.get_mut(entry + 1) += 10. * self.rigid_parameters.brownian_amplitude * gy;
            *state.get_mut(entry + 2) += 10. * self.rigid_parameters.brownian_amplitude * gz;
            if let ShakeTarget::Helix(_) = nucl {
                let delta_roll =
                    rnd.gen::<f32>() * 2. * std::f32::consts::PI - std::f32::consts::PI;
                let mut iterator = state.iter().skip(entry + 3);
                let rotation = Rotor3::new(
                    *iterator.next().unwrap(),
                    Bivec3::new(
                        *iterator.next().unwrap(),
                        *iterator.next().unwrap(),
                        *iterator.next().unwrap(),
                    ),
                )
                .normalized();
                let rotation = rotation * Rotor3::from_rotation_yz(delta_roll);
                let mut iterator = state.iter_mut().skip(entry + 3);
                *iterator.next().unwrap() = rotation.s;
                *iterator.next().unwrap() = rotation.bv.xy;
                *iterator.next().unwrap() = rotation.bv.xz;
                *iterator.next().unwrap() = rotation.bv.yz;
            }
        }
    }
}

impl ExplicitODE<f32> for HelixSystem {
    // We read the sytem in the following format. For each grid, we read
    // * 3 f32 for position
    // * 4 f32 for rotation
    // * 3 f32 for linear momentum
    // * 3 f32 for angular momentum

    fn func(&self, _t: &f32, x: &Vector<f32>) -> Vector<f32> {
        let (positions, rotations, linear_momentums, angular_momentums) = self.read_state(x);
        let (forces, torques) = self.forces_and_torques(&positions, &rotations);

        let nb_element = self.helices.len() + self.free_nucls.len();
        let mut ret = Vec::with_capacity(13 * nb_element);
        for i in 0..nb_element {
            if i < self.helices.len() {
                let d_position =
                    linear_momentums[i] / (self.helices[i].height() * self.rigid_parameters.mass);
                ret.push(d_position.x);
                ret.push(d_position.y);
                ret.push(d_position.z);
                let omega = self.helices[i].inertia_inverse * angular_momentums[i]
                    / self.rigid_parameters.mass;
                let d_rotation = 0.5
                    * Rotor3::from_quaternion_array([omega.x, omega.y, omega.z, 0f32])
                    * rotations[i];

                ret.push(d_rotation.s);
                ret.push(d_rotation.bv.xy);
                ret.push(d_rotation.bv.xz);
                ret.push(d_rotation.bv.yz);

                let d_linear_momentum = forces[i]
                    - linear_momentums[i] * self.rigid_parameters.k_friction
                        / (self.helices[i].height() * self.rigid_parameters.mass);

                ret.push(d_linear_momentum.x);
                ret.push(d_linear_momentum.y);
                ret.push(d_linear_momentum.z);

                let d_angular_momentum = torques[i]
                    - angular_momentums[i] * self.rigid_parameters.k_friction
                        / (self.helices[i].height() * self.rigid_parameters.mass);
                ret.push(d_angular_momentum.x);
                ret.push(d_angular_momentum.y);
                ret.push(d_angular_momentum.z);
            } else {
                let d_position = linear_momentums[i] / (self.rigid_parameters.mass / 2.);
                ret.push(d_position.x);
                ret.push(d_position.y);
                ret.push(d_position.z);

                let d_rotation = Rotor3::from_quaternion_array([0., 0., 0., 0.]);
                ret.push(d_rotation.s);
                ret.push(d_rotation.bv.xy);
                ret.push(d_rotation.bv.xz);
                ret.push(d_rotation.bv.yz);

                let d_linear_momentum = forces[i]
                    - linear_momentums[i] * self.rigid_parameters.k_friction
                        / (self.rigid_parameters.mass / 2.);

                ret.push(d_linear_momentum.x);
                ret.push(d_linear_momentum.y);
                ret.push(d_linear_momentum.z);

                let d_angular_momentum = torques[i]
                    - angular_momentums[i] * self.rigid_parameters.k_friction
                        / (self.rigid_parameters.mass / 2.);
                ret.push(d_angular_momentum.x);
                ret.push(d_angular_momentum.y);
                ret.push(d_angular_momentum.z);
            }
        }

        Vector::new_row(ret.len(), ret)
    }

    fn time_span(&self) -> (f32, f32) {
        self.time_span
    }

    fn init_cond(&self) -> Vector<f32> {
        if let Some(state) = self.last_state.clone() {
            state
        } else {
            let nb_iter = self.helices.len() + self.free_nucls.len();
            let mut ret = Vec::with_capacity(13 * nb_iter);
            for i in 0..self.helices.len() {
                let position = self.helices[i].center_of_mass();
                ret.push(position.x);
                ret.push(position.y);
                ret.push(position.z);
                let rotation = self.helices[i].orientation;

                ret.push(rotation.s);
                ret.push(rotation.bv.xy);
                ret.push(rotation.bv.xz);
                ret.push(rotation.bv.yz);

                let linear_momentum = Vec3::zero();

                ret.push(linear_momentum.x);
                ret.push(linear_momentum.y);
                ret.push(linear_momentum.z);

                let angular_momentum = Vec3::zero();
                ret.push(angular_momentum.x);
                ret.push(angular_momentum.y);
                ret.push(angular_momentum.z);
            }
            for pos in self.free_nucl_position.iter() {
                ret.push(pos.x);
                ret.push(pos.y);
                ret.push(pos.z);

                let rotation = Rotor3::identity();
                ret.push(rotation.s);
                ret.push(rotation.bv.xy);
                ret.push(rotation.bv.xz);
                ret.push(rotation.bv.yz);

                let linear_momentum = Vec3::zero();

                ret.push(linear_momentum.x);
                ret.push(linear_momentum.y);
                ret.push(linear_momentum.z);

                let angular_momentum = Vec3::zero();
                ret.push(angular_momentum.x);
                ret.push(angular_momentum.y);
                ret.push(angular_momentum.z);
            }
            Vector::new_row(ret.len(), ret)
        }
    }
}

struct GridsSystem {
    springs: Vec<(ApplicationPoint, ApplicationPoint)>,
    grids: Vec<RigidGrid>,
    time_span: (f32, f32),
    last_state: Option<Vector<f32>>,
    #[allow(dead_code)]
    anchors: Vec<(ApplicationPoint, Vec3)>,
}

impl GridsSystem {
    fn forces_and_torques(
        &self,
        positions: &[Vec3],
        orientations: &[Rotor3],
        _volume_exclusion: f32,
    ) -> (Vec<Vec3>, Vec<Vec3>) {
        let mut forces = vec![Vec3::zero(); self.grids.len()];
        let mut torques = vec![Vec3::zero(); self.grids.len()];

        const L0: f32 = 0.7;
        const K_SPRING: f32 = 1.;

        let point_conversion = |application_point: &ApplicationPoint| {
            let g_id = application_point.grid_id;
            let position = positions[g_id];
            let orientation = orientations[g_id];
            application_point.position_on_grid.rotated_by(orientation) + position
        };

        for spring in self.springs.iter() {
            let point_0 = point_conversion(&spring.0);
            let point_1 = point_conversion(&spring.1);
            let len = (point_1 - point_0).mag();
            //println!("len {}", len);
            let norm = len - L0;

            // The force applied on point 0
            let force = K_SPRING * norm * (point_1 - point_0) / len;

            forces[spring.0.grid_id] += force;
            forces[spring.1.grid_id] -= force;

            let torque0 = (point_0 - positions[spring.0.grid_id]).cross(force);
            let torque1 = (point_1 - positions[spring.1.grid_id]).cross(-force);

            torques[spring.0.grid_id] += torque0;
            torques[spring.1.grid_id] += torque1;
        }
        /*
        for i in 0..self.grids.len() {
            for j in (i + 1)..self.grids.len() {
                let grid_1 = &self.grids[i];
                let grid_2 = &self.grids[j];
                for h1 in grid_1.helices.iter() {
                    let a = Vec3::new(h1.x_min, h1.y_pos, h1.z_pos);
                    let a = a.rotated_by(orientations[i]) + positions[i];
                    let b = Vec3::new(h1.x_max, h1.y_pos, h1.z_pos);
                    let b = b.rotated_by(orientations[i]) + positions[i];
                    for h2 in grid_2.helices.iter() {
                        let c = Vec3::new(h2.x_min, h2.y_pos, h2.z_pos);
                        let c = c.rotated_by(orientations[j]) + positions[j];
                        let d = Vec3::new(h2.x_max, h2.y_pos, h2.z_pos);
                        let d = d.rotated_by(orientations[j]) + positions[j];
                        let r = 2.;
                        let (dist, vec, point_a, point_c) = distance_segment(a, b, c, d);
                        if dist < r {
                            let norm = ((dist - r) / dist).powi(2) / 1. * 1000.;
                            forces[i] += norm * vec;
                            forces[j] += -norm * vec;
                            let torque0 = (point_a - positions[i]).cross(norm * vec);
                            let torque1 = (point_c - positions[j]).cross(-norm * vec);
                            torques[i] += torque0;
                            torques[j] += torque1;
                        }
                    }
                }
            }
        }*/

        (forces, torques)
    }
}

impl ExplicitODE<f32> for GridsSystem {
    // We read the sytem in the following format. For each grid, we read
    // * 3 f32 for position
    // * 4 f32 for rotation
    // * 3 f32 for linear momentum
    // * 3 f32 for angular momentum

    fn func(&self, _t: &f32, x: &Vector<f32>) -> Vector<f32> {
        let (positions, rotations, linear_momentums, angular_momentums) = self.read_state(x);
        let volume_exclusion = 1.;
        let (forces, torques) = self.forces_and_torques(&positions, &rotations, volume_exclusion);

        let mut ret = Vec::with_capacity(13 * self.grids.len());
        for i in 0..self.grids.len() {
            let d_position = linear_momentums[i] / self.grids[i].mass;
            ret.push(d_position.x);
            ret.push(d_position.y);
            ret.push(d_position.z);
            let omega = self.grids[i].inertia_inverse * angular_momentums[i];
            let d_rotation = 0.5
                * Rotor3::from_quaternion_array([omega.x, omega.y, omega.z, 0f32])
                * rotations[i];

            ret.push(d_rotation.s);
            ret.push(d_rotation.bv.xy);
            ret.push(d_rotation.bv.xz);
            ret.push(d_rotation.bv.yz);

            let d_linear_momentum = forces[i] - linear_momentums[i] * 100. / self.grids[i].mass;

            ret.push(d_linear_momentum.x);
            ret.push(d_linear_momentum.y);
            ret.push(d_linear_momentum.z);

            let d_angular_momentum = torques[i] - angular_momentums[i];
            ret.push(d_angular_momentum.x);
            ret.push(d_angular_momentum.y);
            ret.push(d_angular_momentum.z);
        }

        Vector::new_row(ret.len(), ret)
    }

    fn time_span(&self) -> (f32, f32) {
        self.time_span
    }

    fn init_cond(&self) -> Vector<f32> {
        if let Some(state) = self.last_state.clone() {
            state
        } else {
            let mut ret = Vec::with_capacity(13 * self.grids.len());
            for i in 0..self.grids.len() {
                let position = self.grids[i].center_of_mass;
                ret.push(position.x);
                ret.push(position.y);
                ret.push(position.z);
                let rotation = self.grids[i].orientation;

                ret.push(rotation.s);
                ret.push(rotation.bv.xy);
                ret.push(rotation.bv.xz);
                ret.push(rotation.bv.yz);

                let linear_momentum = Vec3::zero();

                ret.push(linear_momentum.x);
                ret.push(linear_momentum.y);
                ret.push(linear_momentum.z);

                let angular_momentum = Vec3::zero();
                ret.push(angular_momentum.x);
                ret.push(angular_momentum.y);
                ret.push(angular_momentum.z);
            }
            Vector::new_row(ret.len(), ret)
        }
    }
}

impl GridsSystem {
    fn read_state(&self, x: &Vector<f32>) -> (Vec<Vec3>, Vec<Rotor3>, Vec<Vec3>, Vec<Vec3>) {
        let mut positions = Vec::with_capacity(self.grids.len());
        let mut rotations = Vec::with_capacity(self.grids.len());
        let mut linear_momentums = Vec::with_capacity(self.grids.len());
        let mut angular_momentums = Vec::with_capacity(self.grids.len());
        let mut iterator = x.iter();
        for _ in 0..self.grids.len() {
            let position = Vec3::new(
                *iterator.next().unwrap(),
                *iterator.next().unwrap(),
                *iterator.next().unwrap(),
            );
            let rotation = Rotor3::new(
                *iterator.next().unwrap(),
                Bivec3::new(
                    *iterator.next().unwrap(),
                    *iterator.next().unwrap(),
                    *iterator.next().unwrap(),
                ),
            )
            .normalized();
            let linear_momentum = Vec3::new(
                *iterator.next().unwrap(),
                *iterator.next().unwrap(),
                *iterator.next().unwrap(),
            );
            let angular_momentum = Vec3::new(
                *iterator.next().unwrap(),
                *iterator.next().unwrap(),
                *iterator.next().unwrap(),
            );
            positions.push(position);
            rotations.push(rotation);
            linear_momentums.push(linear_momentum);
            angular_momentums.push(angular_momentum);
        }
        (positions, rotations, linear_momentums, angular_momentums)
    }
}

#[derive(Debug)]
struct ApplicationPoint {
    grid_id: usize,
    position_on_grid: Vec3,
}

#[derive(Debug)]
struct RigidHelix {
    pub roll: f32,
    pub orientation: Rotor3,
    pub inertia_inverse: Mat3,
    pub center_of_mass: Vec3,
    pub center_to_origin: Vec3,
    pub mass: f32,
    pub id: usize,
    interval: (isize, isize),
}

impl RigidHelix {
    fn new_from_grid(
        y_pos: f32,
        z_pos: f32,
        x_min: f32,
        x_max: f32,
        roll: f32,
        orientation: Rotor3,
        interval: (isize, isize),
    ) -> RigidHelix {
        Self {
            roll,
            orientation,
            center_of_mass: Vec3::new((x_min + x_max) / 2., y_pos, z_pos),
            center_to_origin: -(x_min + x_max) / 2. * Vec3::unit_x(),
            mass: x_max - x_min,
            inertia_inverse: inertia_helix(x_max - x_min, 1.).inversed(),
            // at the moment we do not care for the id when creating a rigid helix for a grid
            id: 0,
            interval,
        }
    }

    fn new_from_world(
        y_pos: f32,
        z_pos: f32,
        x_pos: f32,
        delta: Vec3,
        mass: f32,
        roll: f32,
        orientation: Rotor3,
        id: usize,
        interval: (isize, isize),
    ) -> RigidHelix {
        Self {
            roll,
            orientation,
            center_of_mass: Vec3::new(x_pos, y_pos, z_pos),
            center_to_origin: delta,
            mass,
            inertia_inverse: inertia_helix(mass, 1.).inversed(),
            id,
            interval,
        }
    }

    fn center_of_mass(&self) -> Vec3 {
        self.center_of_mass
    }

    fn height(&self) -> f32 {
        self.mass
    }
}

#[derive(Debug)]
struct RigidGrid {
    /// Center of mass of of the grid in world coordinates
    center_of_mass: Vec3,
    /// Center of mass of the grid in the grid coordinates
    center_of_mass_from_grid: Vec3,
    /// Orientation of the grid in the world coordinates
    orientation: Rotor3,
    inertia_inverse: Mat3,
    mass: f32,
    id: usize,
    helices: Vec<RigidHelix>,
}

impl RigidGrid {
    pub fn from_helices(
        id: usize,
        helices: Vec<RigidHelix>,
        position_grid: Vec3,
        orientation: Rotor3,
    ) -> Self {
        // Center of mass in the grid coordinates.
        println!("helices {:?}", helices);
        let center_of_mass = center_of_mass_helices(&helices);

        // Inertia matrix when the orientation is the identity
        let inertia_matrix = inertia_helices(&helices, center_of_mass);
        let inertia_inverse = inertia_matrix.inversed();
        let mass = helices.iter().map(|h| h.height()).sum();
        Self {
            center_of_mass: center_of_mass.rotated_by(orientation) + position_grid,
            center_of_mass_from_grid: center_of_mass,
            inertia_inverse,
            orientation,
            mass,
            id,
            helices,
        }
    }
}

/// Inertia matrix of an helix of axis e_x, radius r, height h with respect to its center of mass.
fn inertia_helix(h: f32, r: f32) -> Mat3 {
    // The mass is proportinal to the height of the cylinder times its radius squared, we assume that all
    // the cylinder that we work with have the same density
    let m = h * r * r;
    let c = m * r * r / 2.;
    let a = m * (r * r / 4. + h * h / 12.);
    Mat3::new(c * Vec3::unit_x(), a * Vec3::unit_y(), a * Vec3::unit_z())
}

fn center_of_mass_helices(helices: &[RigidHelix]) -> Vec3 {
    let mut total_mass = 0f32;
    let mut ret = Vec3::zero();
    for h in helices.iter() {
        ret += h.center_of_mass() * h.height();
        total_mass += h.height();
    }
    ret / total_mass
}

/// The Inertia matrix of a point with respect to the origin
fn inertia_point(point: Vec3) -> Mat3 {
    Mat3::new(
        Vec3::new(
            point.y * point.y + point.z + point.z,
            -point.x * point.y,
            -point.x * point.z,
        ),
        Vec3::new(
            -point.y * point.x,
            point.x * point.x + point.z * point.z,
            -point.y * point.z,
        ),
        Vec3::new(
            -point.z * point.x,
            -point.z * point.y,
            point.x * point.x + point.y * point.y,
        ),
    )
}

fn inertia_helices(helices: &[RigidHelix], center_of_mass: Vec3) -> Mat3 {
    const HELIX_RADIUS: f32 = 1.;
    let mut ret = Mat3::from_scale(0f32);
    for h in helices.iter() {
        let helix_center = h.center_of_mass();
        let inertia = inertia_helix(h.height(), HELIX_RADIUS);
        ret += inertia_point(helix_center - center_of_mass) * h.height() + inertia;
    }
    ret
}

struct GridsSystemThread {
    grid_system: GridsSystem,
    /// When the wrapped boolean is set to true, stop the simulation perfomed by self.
    stop: Arc<Mutex<bool>>,
    /// When the wrapped option takes the value of some channel, the thread that performs the
    /// simulation sends the last computed state of the system
    sender: Arc<Mutex<Option<Sender<GridSystemState>>>>,
}

impl GridsSystemThread {
    fn new(grid_system: GridsSystem) -> Self {
        Self {
            grid_system,
            stop: Default::default(),
            sender: Default::default(),
        }
    }

    /// Spawn a thread to run the physical simulation. Return a pair of pointers. One to request the
    /// termination of the simulation and one to fetch the current state of the helices.
    fn run(
        mut self,
        computing: Arc<Mutex<bool>>,
    ) -> (
        Arc<Mutex<bool>>,
        Arc<Mutex<Option<Sender<GridSystemState>>>>,
    ) {
        let stop = self.stop.clone();
        let sender = self.sender.clone();
        *computing.lock().unwrap() = true;
        std::thread::spawn(move || {
            while !*self.stop.lock().unwrap() {
                if let Some(snd) = self.sender.lock().unwrap().take() {
                    snd.send(self.get_state()).unwrap();
                }
                let solver = Kutta3::new(1e-4f32);
                if let Ok((_, y)) = solver.solve(&self.grid_system) {
                    self.grid_system.last_state = y.last().cloned();
                }
            }
            *computing.lock().unwrap() = false;
        });
        (stop, sender)
    }

    fn get_state(&self) -> GridSystemState {
        let state = self.grid_system.init_cond();
        let (positions, orientations, _, _) = self.grid_system.read_state(&state);
        let ids = self.grid_system.grids.iter().map(|g| g.id).collect();
        let center_of_mass_from_grid = self
            .grid_system
            .grids
            .iter()
            .map(|g| g.center_of_mass_from_grid)
            .collect();
        GridSystemState {
            positions,
            orientations,
            center_of_mass_from_grid,
            ids,
        }
    }
}

struct HelixSystemThread {
    helix_system: HelixSystem,
    /// When the wrapped boolean is set to true, stop the simulation perfomed by self.
    stop: Arc<Mutex<bool>>,
    /// When the wrapped option takes the value of some channel, the thread that performs the
    /// simulation sends the last computed state of the system
    sender: Arc<Mutex<Option<Sender<RigidHelixState>>>>,
    /// A nucleotide to be shaken
    nucl_shake: Arc<Mutex<Option<ShakeTarget>>>,
    parameters_update: Arc<Mutex<Option<RigidBodyConstants>>>,
}

impl HelixSystemThread {
    fn new(helix_system: HelixSystem) -> Self {
        Self {
            helix_system,
            stop: Default::default(),
            sender: Default::default(),
            nucl_shake: Default::default(),
            parameters_update: Default::default(),
        }
    }

    /// Spawn a thread to run the physical simulation. Return a pair of pointers. One to request the
    /// termination of the simulation and one to fetch the current state of the helices.
    fn run(
        mut self,
        computing: Arc<Mutex<bool>>,
    ) -> (
        Arc<Mutex<bool>>,
        Arc<Mutex<Option<Sender<RigidHelixState>>>>,
    ) {
        let stop = self.stop.clone();
        let sender = self.sender.clone();
        *computing.lock().unwrap() = true;
        std::thread::spawn(move || {
            while !*self.stop.lock().unwrap() {
                if let Some(parameters) = self.parameters_update.lock().unwrap().take() {
                    self.helix_system.update_parameters(parameters)
                }
                if let Some(snd) = self.sender.lock().unwrap().take() {
                    snd.send(self.get_state()).unwrap();
                }
                self.helix_system.next_time();
                let solver = ExplicitEuler::new(1e-4f32);
                if self.helix_system.rigid_parameters.brownian_motion {
                    self.helix_system.brownian_jump();
                }
                if let Some(nucl) = self.nucl_shake.lock().unwrap().take() {
                    self.helix_system.shake_nucl(nucl)
                }
                if let Ok((_, y)) = solver.solve(&self.helix_system) {
                    self.helix_system.last_state = y.last().cloned();
                }
            }
            *computing.lock().unwrap() = false;
        });
        (stop, sender)
    }

    fn get_param_ptr(&self) -> Arc<Mutex<Option<RigidBodyConstants>>> {
        self.parameters_update.clone()
    }

    fn get_nucl_ptr(&self) -> Arc<Mutex<Option<ShakeTarget>>> {
        self.nucl_shake.clone()
    }

    fn get_state(&self) -> RigidHelixState {
        let state = self.helix_system.init_cond();
        let (positions, orientations, _, _) = self.helix_system.read_state(&state);
        let ids = self.helix_system.helices.iter().map(|g| g.id).collect();
        let center_of_mass_from_helix = self
            .helix_system
            .helices
            .iter()
            .map(|h| h.center_to_origin)
            .collect();
        RigidHelixState {
            positions,
            orientations,
            center_of_mass_from_helix,
            ids,
        }
    }
}

#[derive(Clone)]
pub struct GridSystemState {
    positions: Vec<Vec3>,
    orientations: Vec<Rotor3>,
    center_of_mass_from_grid: Vec<Vec3>,
    ids: Vec<usize>,
}

pub(super) struct RigidBodyPtr {
    stop: Arc<Mutex<bool>>,
    state: Arc<Mutex<Option<Sender<GridSystemState>>>>,
    instant: Instant,
}

pub(super) struct RigidHelixPtr {
    stop: Arc<Mutex<bool>>,
    state: Arc<Mutex<Option<Sender<RigidHelixState>>>>,
    shake_nucl: Arc<Mutex<Option<ShakeTarget>>>,
    instant: Instant,
}

#[derive(Debug, Clone)]
pub struct RigidHelixState {
    positions: Vec<Vec3>,
    orientations: Vec<Rotor3>,
    center_of_mass_from_helix: Vec<Vec3>,
    ids: Vec<usize>,
}

pub(super) struct RigidHelixSimulator {
    nucl_maps: HashMap<Nucl, FreeNucl>,
    free_nucls_ids: HashMap<FreeNucl, usize>,
    roll: Vec<f32>,
    nb_helices: usize,
    simulation_ptr: RigidHelixPtr,
    state_update: Option<RigidHelixState>,
    parameters: Parameters,
    rigid_parameters: Arc<Mutex<Option<RigidBodyConstants>>>,
    initial_state: RigidHelixState,
}

impl RigidHelixSimulator {
    fn start_simulation(
        helix_system: HelixSystem,
        computing: Arc<Mutex<bool>>,
        interval_results: IntervalResult,
    ) -> Self {
        let roll = helix_system.helices.iter().map(|h| h.roll).collect();
        let parameters = helix_system.parameters.clone();
        let helix_system_thread = HelixSystemThread::new(helix_system);
        let rigid_parameters = helix_system_thread.get_param_ptr();
        let shake_nucl = helix_system_thread.get_nucl_ptr();

        let date = Instant::now();
        let initial_state = helix_system_thread.get_state();
        let (stop, snd) = helix_system_thread.run(computing);
        let simulation_ptr = RigidHelixPtr {
            instant: date,
            stop,
            shake_nucl,
            state: snd,
        };
        Self {
            roll,
            parameters,
            nucl_maps: interval_results.nucl_map,
            free_nucls_ids: interval_results.free_nucl_ids,
            nb_helices: interval_results.intervals.len(),
            simulation_ptr,
            state_update: None,
            rigid_parameters,
            initial_state,
        }
    }

    pub(super) fn update_parameters(&mut self, rigid_parameters: RigidBodyConstants) {
        *self.rigid_parameters.lock().unwrap() = Some(rigid_parameters);
    }

    pub(super) fn shake_nucl(&mut self, nucl: Nucl) {
        if let Some(free_nucl) = self.nucl_maps.get(&nucl) {
            let shake_target = if let Some(helix) = free_nucl.helix {
                Some(ShakeTarget::Helix(helix))
            } else {
                self.free_nucls_ids
                    .get(free_nucl)
                    .map(|id| ShakeTarget::FreeNucl(*id))
            };
            *self.simulation_ptr.shake_nucl.lock().unwrap() = shake_target
        }
    }

    fn check_simulation(&mut self) {
        let now = Instant::now();
        if (now - self.simulation_ptr.instant).as_millis() > 30 {
            let (snd, rcv) = std::sync::mpsc::channel();
            *self.simulation_ptr.state.lock().unwrap() = Some(snd);
            self.state_update = rcv.recv().ok();
            /*
            for i in 0..state.ids.len() {
                let position = state.positions[i];
                let orientation = state.orientations[i].normalized();
                self.design.helices.get_mut(&state.ids[i]).unwrap().position =
                    position + state.center_of_mass_from_helix[i].rotated_by(orientation);
                self.design
                    .helices
                    .get_mut(&state.ids[i])
                    .unwrap()
                    .orientation = orientation;
                }
            */
            self.simulation_ptr.instant = now;
        }
    }

    fn update_positions(
        &mut self,
        identifier_nucl: &HashMap<Nucl, u32, RandomState>,
        space_position: &mut HashMap<u32, [f32; 3], RandomState>,
    ) -> bool {
        if let Some(state) = self.state_update.take() {
            let helices: Vec<Helix> = (0..self.nb_helices)
                .map(|n| {
                    let orientation = state.orientations[n].normalized();
                    let position = state.positions[n]
                        + state.center_of_mass_from_helix[n].rotated_by(orientation);
                    let mut h = Helix::new(position, orientation);
                    h.roll(self.roll[n]);
                    h
                })
                .collect();
            for (nucl, id) in identifier_nucl.iter() {
                let free_nucl = self.nucl_maps[nucl];
                if let Some(n) = free_nucl.helix {
                    space_position.insert(
                        *id,
                        helices[n]
                            .space_pos(&self.parameters, free_nucl.position, free_nucl.forward)
                            .into(),
                    );
                } else {
                    let free_id = self.free_nucls_ids[&free_nucl];
                    space_position.insert(*id, state.positions[self.nb_helices + free_id].into());
                }
            }
            true
        } else {
            false
        }
    }
}

impl Data {
    /*
    pub fn grid_simulation(&mut self, time_span: (f32, f32)) {
        if let Some(grid_system) = self.make_grid_system(time_span) {
            let solver = Kutta3::new(1e-4f32);
            if let Ok((_, y)) = solver.solve(&grid_system) {
                let last_state = y.last().unwrap();
                let (positions, rotations, _, _) = grid_system.read_state(last_state);
                for (i, rigid_grid) in grid_system.grids.iter().enumerate() {
                    let position = positions[i];
                    let orientation = rotations[i].normalized();
                    self.grid_manager.grids[rigid_grid.id].position =
                        position - rigid_grid.center_of_mass_from_grid.rotated_by(orientation);
                    self.grid_manager.grids[rigid_grid.id].orientation = orientation;
                }
                self.grid_manager.update(&mut self.design);
                self.hash_maps_update = true;
                self.update_status = true;
            } else {
                println!("error while solving");
            }
        } else {
            println!("could not make grid system");
        }
    }
    */

    /*
    pub fn helices_simulation(&mut self, time_span: (f32, f32)) {
        if let Some(helix_system) = self.make_helices_system(time_span) {
            let solver = Kutta3::new(1e-4f32);
            if let Ok((_, y)) = solver.solve(&helix_system) {
                let last_state = y.last().unwrap();
                let (positions, rotations, _, _) = helix_system.read_state(last_state);
                for (i, rigid_helix) in helix_system.helices.iter().enumerate() {
                    let position = positions[i];
                    let orientation = rotations[i].normalized();
                    let helix = self.design.helices.get_mut(&rigid_helix.id).unwrap();
                    helix.position = position - rigid_helix.center_of_mass;
                    helix.orientation = orientation;
                    helix.end_movement();
                }
                self.hash_maps_update = true;
                self.update_status = true;
            } else {
                println!("error while solving");
            }
        } else {
            println!("could not make grid system");
        }
    }*/

    fn make_flexible_helices_system(
        &self,
        time_span: (f32, f32),
        interval_results: &IntervalResult,
        rigid_parameters: RigidBodyConstants,
    ) -> Option<HelixSystem> {
        let parameters = self.design.parameters.unwrap_or_default();
        let mut rigid_helices = Vec::with_capacity(interval_results.helix_map.len());
        for i in 0..interval_results.helix_map.len() {
            let h_id = interval_results.helix_map[i];
            let interval = interval_results.intervals[i];
            let rigid_helix = self.make_rigid_helix_world_pov_interval(h_id, interval, &parameters);
            rigid_helices.push(rigid_helix);
        }
        let xovers = self.get_xovers_list();
        let mut springs = Vec::with_capacity(xovers.len());
        let mut mixed_springs = Vec::with_capacity(xovers.len());
        let mut free_springs = Vec::with_capacity(xovers.len());
        for (_, (n1, n2)) in xovers {
            println!("{:?}", (n1, n2));
            let free_nucl1 = interval_results.nucl_map[&n1];
            let free_nucl2 = interval_results.nucl_map[&n2];
            if let Some((h1, h2)) = free_nucl1.helix.zip(free_nucl2.helix) {
                let rigid_1 = RigidNucl {
                    helix: h1,
                    position: n1.position,
                    forward: n1.forward,
                };
                let rigid_2 = RigidNucl {
                    helix: h2,
                    position: n2.position,
                    forward: n2.forward,
                };
                springs.push((rigid_1, rigid_2));
            }
        }
        for (n1, n2) in self.nucleotides_involved.values() {
            let free_nucl1 = interval_results.nucl_map[&n1];
            let free_nucl2 = interval_results.nucl_map[&n2];
            if let Some((_, _)) = free_nucl1.helix.zip(free_nucl2.helix) {
                // Do nothing, this case has either been handled in the xover loop
                // or this bound is rigid
            } else if let Some(h1) = free_nucl1.helix {
                let rigid_1 = RigidNucl {
                    helix: h1,
                    position: n1.position,
                    forward: n1.forward,
                };
                let free_id = interval_results.free_nucl_ids[&free_nucl2];
                mixed_springs.push((rigid_1, free_id));
            } else if let Some(h2) = free_nucl2.helix {
                let rigid_2 = RigidNucl {
                    helix: h2,
                    position: n2.position,
                    forward: n2.forward,
                };
                let free_id = interval_results.free_nucl_ids[&free_nucl1];
                mixed_springs.push((rigid_2, free_id));
            } else {
                let free_id_1 = interval_results.free_nucl_ids[&free_nucl1];
                let free_id_2 = interval_results.free_nucl_ids[&free_nucl2];
                free_springs.push((free_id_1, free_id_2));
            }
        }
        let mut anchors = vec![];
        let mut free_anchors = vec![];
        for anchor in self.anchors.iter() {
            if let Some(n_id) = self.identifier_nucl.get(anchor) {
                let position: Vec3 = self.space_position[n_id].into();
                if let Some(free_nucl) = interval_results.nucl_map.get(anchor) {
                    if let Some(rigid_helix) = free_nucl.helix {
                        let rigid_nucl = RigidNucl {
                            helix: rigid_helix,
                            position: anchor.position,
                            forward: anchor.forward,
                        };
                        anchors.push((rigid_nucl, position));
                    } else if let Some(id) = interval_results.free_nucl_ids.get(free_nucl) {
                        free_anchors.push((*id, position));
                    }
                }
            }
        }
        let mut rnd = rand::thread_rng();
        let mut brownian_heap = BinaryHeap::new();
        let exp_law = Exp::new(rigid_parameters.brownian_rate).unwrap();
        for i in 0..interval_results.free_nucls.len() {
            if !free_anchors.iter().any(|(x, _)| *x == i) {
                let t = rnd.sample(exp_law);
                brownian_heap.push((Reverse(t.into()), i));
            }
        }
        Some(HelixSystem {
            helices: rigid_helices,
            springs,
            mixed_springs,
            free_springs,
            free_nucls: interval_results.free_nucls.clone(),
            free_nucl_position: interval_results.free_nucl_position.clone(),
            last_state: None,
            time_span,
            parameters,
            anchors,
            free_anchors,
            brownian_heap,
            current_time: 0.,
            next_time: 0.,
            rigid_parameters,
            max_time_step: time_span.1,
        })
    }

    fn make_grid_system(
        &self,
        time_span: (f32, f32),
        _paramaters: RigidBodyConstants,
    ) -> Option<GridsSystem> {
        let intervals = self.design.get_intervals();
        let parameters = self.design.parameters.unwrap_or_default();
        let mut selected_grids = HashMap::with_capacity(self.grid_manager.grids.len());
        let mut rigid_grids = Vec::with_capacity(self.grid_manager.grids.len());
        for g_id in 0..self.grid_manager.grids.len() {
            if let Some(rigid_grid) = self.make_rigid_grid(g_id, &intervals, &parameters) {
                selected_grids.insert(g_id, rigid_grids.len());
                rigid_grids.push(rigid_grid);
            }
        }
        if rigid_grids.len() == 0 {
            return None;
        }
        let xovers = self.get_xovers_list();
        let mut springs = Vec::new();
        for (_, (n1, n2)) in xovers {
            let h1 = self.design.helices.get(&n1.helix)?;
            let h2 = self.design.helices.get(&n2.helix)?;
            let g_id1 = h1.grid_position.map(|gp| gp.grid);
            let g_id2 = h2.grid_position.map(|gp| gp.grid);
            if let Some((g_id1, g_id2)) = g_id1.zip(g_id2) {
                if g_id1 != g_id2 {
                    let rigid_id1 = selected_grids.get(&g_id1).cloned();
                    let rigid_id2 = selected_grids.get(&g_id2).cloned();
                    if let Some((rigid_id1, rigid_id2)) = rigid_id1.zip(rigid_id2) {
                        let grid1 = &self.grid_manager.grids[g_id1];
                        let grid2 = &self.grid_manager.grids[g_id2];
                        let pos1 = (h1.space_pos(&parameters, n1.position, n1.forward)
                            - rigid_grids[rigid_id1].center_of_mass)
                            .rotated_by(grid1.orientation.reversed());
                        let pos2 = (h2.space_pos(&parameters, n2.position, n2.forward)
                            - rigid_grids[rigid_id2].center_of_mass)
                            .rotated_by(grid2.orientation.reversed());
                        let application_point1 = ApplicationPoint {
                            position_on_grid: pos1,
                            grid_id: rigid_id1,
                        };
                        let application_point2 = ApplicationPoint {
                            position_on_grid: pos2,
                            grid_id: rigid_id2,
                        };
                        springs.push((application_point1, application_point2));
                    }
                }
            }
        }
        Some(GridsSystem {
            springs,
            grids: rigid_grids,
            time_span,
            last_state: None,
            anchors: vec![],
        })
    }

    fn make_rigid_grid(
        &self,
        g_id: usize,
        intervals: &BTreeMap<usize, (isize, isize)>,
        parameters: &Parameters,
    ) -> Option<RigidGrid> {
        let helices: Vec<usize> = self.grids[g_id]
            .read()
            .unwrap()
            .helices()
            .values()
            .cloned()
            .collect();
        let grid = self.grid_manager.grids.get(g_id)?;
        let mut rigid_helices = Vec::with_capacity(helices.len());
        for h in helices {
            if let Some(rigid_helix) = self.make_rigid_helix_grid_pov(h, intervals, parameters) {
                rigid_helices.push(rigid_helix)
            }
        }
        if rigid_helices.len() > 0 {
            Some(RigidGrid::from_helices(
                g_id,
                rigid_helices,
                grid.position,
                grid.orientation,
            ))
        } else {
            None
        }
    }

    fn make_rigid_helix_grid_pov(
        &self,
        h_id: usize,
        intervals: &BTreeMap<usize, (isize, isize)>,
        parameters: &Parameters,
    ) -> Option<RigidHelix> {
        let (x_min, x_max) = intervals.get(&h_id)?;
        let helix = self.design.helices.get(&h_id)?;
        let grid_position = helix.grid_position?;
        let grid = self.grid_manager.grids.get(grid_position.grid)?;
        let position = grid.position_helix(grid_position.x, grid_position.y) - grid.position;
        Some(RigidHelix::new_from_grid(
            position.y,
            position.z,
            *x_min as f32 * parameters.z_step,
            *x_max as f32 * parameters.z_step,
            helix.roll,
            helix.orientation,
            (*x_min, *x_max),
        ))
    }

    fn make_rigid_helix_world_pov_interval(
        &self,
        h_id: usize,
        interval: (isize, isize),
        parameters: &Parameters,
    ) -> RigidHelix {
        let (x_min, x_max) = &interval;
        let helix = self.design.helices.get(&h_id).expect("helix");
        let left = helix.axis_position(parameters, *x_min);
        let right = helix.axis_position(parameters, *x_max);
        let position = (left + right) / 2.;
        let position_delta =
            -(*x_max as f32 * parameters.z_step + *x_min as f32 * parameters.z_step) / 2.
                * Vec3::unit_x();
        RigidHelix::new_from_world(
            position.y,
            position.z,
            position.x,
            position_delta,
            (right - left).mag(),
            helix.roll,
            helix.orientation,
            h_id,
            interval,
        )
    }

    pub(super) fn check_rigid_body(&mut self) {
        if let Some(ptrs) = self.rigid_body_ptr.as_mut() {
            let now = Instant::now();
            if (now - ptrs.instant).as_millis() > 30 {
                let (snd, rcv) = std::sync::mpsc::channel();
                *ptrs.state.lock().unwrap() = Some(snd);
                let state = rcv.recv().unwrap();
                ptrs.instant = now;
                self.read_grid_system_state(state);
            }
        }
    }

    fn read_grid_system_state(&mut self, state: GridSystemState) {
        for i in 0..state.ids.len() {
            let position = state.positions[i];
            let orientation = state.orientations[i].normalized();
            let grid = &mut self.grid_manager.grids[state.ids[i]];
            grid.position = position - state.center_of_mass_from_grid[i].rotated_by(orientation);
            grid.orientation = orientation;
            grid.end_movement();
        }
        self.grid_manager.update(&mut self.design);
        self.hash_maps_update = true;
        self.update_status = true;
        self.hash_maps_update = true;
        self.update_status = true;
    }

    pub(super) fn check_rigid_helices(&mut self) {
        if let Some(ptrs) = self.helix_simulation_ptr.as_mut() {
            let now = Instant::now();
            if (now - ptrs.instant).as_millis() > 30 {
                let (snd, rcv) = std::sync::mpsc::channel();
                *ptrs.state.lock().unwrap() = Some(snd);
                let state = rcv.recv().unwrap();
                ptrs.instant = now;
                self.read_rigid_helix_state(state);
            }
        }
    }

    fn read_rigid_helix_state(&mut self, state: RigidHelixState) {
        for i in 0..state.ids.len() {
            let position = state.positions[i];
            let orientation = state.orientations[i].normalized();
            self.design.helices.get_mut(&state.ids[i]).unwrap().position =
                position + state.center_of_mass_from_helix[i].rotated_by(orientation);
            self.design
                .helices
                .get_mut(&state.ids[i])
                .unwrap()
                .orientation = orientation;
        }
        self.hash_maps_update = true;
        self.update_status = true;
        self.hash_maps_update = true;
        self.update_status = true;
    }

    pub(super) fn read_rigid_helix_update(&mut self) -> bool {
        if let Some(simulator) = self.rigid_helix_simulator.as_mut() {
            simulator.check_simulation();
            simulator.update_positions(&self.identifier_nucl, &mut self.space_position)
        } else {
            false
        }
    }

    pub fn undo_grid_simulation(&mut self, initial_state: GridSystemState) {
        self.stop_rigid_body();
        self.read_grid_system_state(initial_state);
    }

    pub fn rigid_body_request(
        &mut self,
        request: (f32, f32),
        computing: Arc<Mutex<bool>>,
        parameters: RigidBodyConstants,
    ) -> Option<GridSystemState> {
        if self.rigid_body_ptr.is_some() {
            self.stop_rigid_body();
            None
        } else {
            self.before_simul_save();
            self.start_rigid_body(request, computing, parameters)
        }
    }

    pub fn undo_helix_simulation(&mut self, initial_state: RigidHelixState) {
        self.stop_free_helix_simulation();
        self.read_rigid_helix_state(initial_state);
    }

    pub fn helix_simulation_request(
        &mut self,
        request: (f32, f32),
        computing: Arc<Mutex<bool>>,
        parameters: RigidBodyConstants,
    ) -> Option<RigidHelixState> {
        /*
        if self.helix_simulation_ptr.is_some() {
            self.stop_helix_simulation()
        } else {
            self.start_helix_simulation(request, computing)
        }
        */
        if self.rigid_helix_simulator.is_some() {
            self.stop_free_helix_simulation();
            None
        } else {
            self.start_free_helix_simulation(request, computing, parameters)
        }
    }

    fn start_rigid_body(
        &mut self,
        request: (f32, f32),
        computing: Arc<Mutex<bool>>,
        parameters: RigidBodyConstants,
    ) -> Option<GridSystemState> {
        if let Some(grid_system) = self.make_grid_system(request, parameters) {
            let grid_system_thread = GridsSystemThread::new(grid_system);
            let date = Instant::now();
            let initial_state = grid_system_thread.get_state();
            let (stop, snd) = grid_system_thread.run(computing);
            self.rigid_body_ptr = Some(RigidBodyPtr {
                instant: date,
                stop,
                state: snd,
            });
            Some(initial_state)
        } else {
            None
        }
    }

    fn stop_rigid_body(&mut self) {
        if let Some(rigid_body_ptr) = self.rigid_body_ptr.as_mut() {
            *rigid_body_ptr.stop.lock().unwrap() = true;
        } else {
            println!("design was not performing rigid body simulation");
        }
        self.rigid_body_ptr = None;
    }

    /*
    fn start_helix_simulation(&mut self, request: (f32, f32), computing: Arc<Mutex<bool>>) {
        let interval_results = self.read_intervals();
        let helix_system_opt = self.make_flexible_helices_system(request, &interval_results, parameters);
        if let Some(helix_system) = helix_system_opt {
            let grid_system_thread = HelixSystemThread::new(helix_system);
            let date = Instant::now();
            let (stop, snd) = grid_system_thread.run(computing);
            self.helix_simulation_ptr = Some(RigidHelixPtr {
                instant: date,
                stop,
                state: snd,
            });
        }
    }*/

    fn start_free_helix_simulation(
        &mut self,
        request: (f32, f32),
        computing: Arc<Mutex<bool>>,
        parameters: RigidBodyConstants,
    ) -> Option<RigidHelixState> {
        let interval_results = self.read_intervals();
        let helix_system_opt =
            self.make_flexible_helices_system(request, &interval_results, parameters);
        if let Some(helix_system) = helix_system_opt {
            let helix_simulator =
                RigidHelixSimulator::start_simulation(helix_system, computing, interval_results);
            let ret = helix_simulator.initial_state.clone();
            self.rigid_helix_simulator = Some(helix_simulator);
            Some(ret)
        } else {
            None
        }
    }

    fn stop_free_helix_simulation(&mut self) {
        if let Some(helix_simulator) = self.rigid_helix_simulator.as_mut() {
            *helix_simulator.simulation_ptr.stop.lock().unwrap() = true;
        } else {
            println!("design was not performing rigid body simulation");
        }
        self.rigid_helix_simulator = None;
    }

    pub(super) fn stop_simulations(&mut self) {
        self.stop_free_helix_simulation();
        self.stop_rigid_body();
    }

    pub fn read_intervals(&self) -> IntervalResult {
        // TODO remove pub after testing
        let mut nucl_map = HashMap::new();
        let mut current_helix = None;
        let mut helix_map = Vec::new();
        let mut free_nucls = Vec::new();
        let mut free_nucl_ids = HashMap::new();
        let mut free_nucl_position = Vec::new();
        let mut intervals = Vec::new();
        for s in self.design.strands.values() {
            for d in s.domains.iter() {
                println!("New dom");
                if let Some(nucl) = d.prime5_end() {
                    if !nucl_map.contains_key(&nucl) || !nucl.forward {
                        let starting_doubled = self.identifier_nucl.contains_key(&nucl.compl());
                        let starting_nucl = nucl.clone();
                        let mut prev_doubled = false;
                        let mut moving_nucl = starting_nucl;
                        let mut starting_helix = if starting_doubled {
                            Some(current_helix.clone())
                        } else {
                            None
                        };
                        while self.identifier_nucl.contains_key(&moving_nucl) {
                            println!("nucl {:?}", moving_nucl);
                            let doubled = self.identifier_nucl.contains_key(&moving_nucl.compl());
                            if doubled && nucl.forward {
                                println!("has compl");
                                let helix = if prev_doubled {
                                    current_helix.unwrap()
                                } else {
                                    helix_map.push(nucl.helix);
                                    intervals.push((moving_nucl.position, moving_nucl.position));
                                    if let Some(n) = current_helix.as_mut() {
                                        *n += 1;
                                        *n
                                    } else {
                                        current_helix = Some(0);
                                        0
                                    }
                                };
                                println!("helix {}", helix);
                                nucl_map.insert(
                                    moving_nucl,
                                    FreeNucl::with_helix(&moving_nucl, Some(helix)),
                                );
                                nucl_map.insert(
                                    moving_nucl.compl(),
                                    FreeNucl::with_helix(&moving_nucl.compl(), Some(helix)),
                                );
                                intervals[helix].0 = intervals[helix].0.min(moving_nucl.position);
                                intervals[helix].1 = intervals[helix].1.max(moving_nucl.position);
                            } else if !doubled {
                                println!("has not compl");
                                nucl_map
                                    .insert(moving_nucl, FreeNucl::with_helix(&moving_nucl, None));
                                free_nucl_ids.insert(
                                    FreeNucl::with_helix(&moving_nucl, None),
                                    free_nucls.len(),
                                );
                                free_nucls.push(FreeNucl::with_helix(&moving_nucl, None));
                                let id = self.identifier_nucl[&moving_nucl];
                                free_nucl_position.push(self.space_position[&id].into());
                            }
                            prev_doubled = doubled;
                            moving_nucl = moving_nucl.left();
                        }
                        prev_doubled = starting_doubled;
                        moving_nucl = starting_nucl.right();
                        while self.identifier_nucl.contains_key(&moving_nucl) {
                            println!("nucl {:?}", moving_nucl);
                            let doubled = self.identifier_nucl.contains_key(&moving_nucl.compl());
                            if doubled && nucl.forward {
                                println!("has compl");
                                let helix = if prev_doubled {
                                    current_helix.unwrap()
                                } else {
                                    if let Some(helix) = starting_helix.take() {
                                        if let Some(n) = helix {
                                            n + 1
                                        } else {
                                            0
                                        }
                                    } else {
                                        helix_map.push(nucl.helix);
                                        intervals
                                            .push((moving_nucl.position, moving_nucl.position));
                                        if let Some(n) = current_helix.as_mut() {
                                            *n += 1;
                                            *n
                                        } else {
                                            current_helix = Some(0);
                                            0
                                        }
                                    }
                                };
                                println!("helix {}", helix);
                                intervals[helix].0 = intervals[helix].0.min(moving_nucl.position);
                                intervals[helix].1 = intervals[helix].1.max(moving_nucl.position);
                                nucl_map.insert(
                                    moving_nucl,
                                    FreeNucl::with_helix(&moving_nucl, Some(helix)),
                                );
                                nucl_map.insert(
                                    moving_nucl.compl(),
                                    FreeNucl::with_helix(&moving_nucl.compl(), Some(helix)),
                                );
                            } else if !doubled {
                                println!("has not compl");
                                nucl_map
                                    .insert(moving_nucl, FreeNucl::with_helix(&moving_nucl, None));
                                free_nucl_ids.insert(
                                    FreeNucl::with_helix(&moving_nucl, None),
                                    free_nucls.len(),
                                );
                                free_nucls.push(FreeNucl::with_helix(&moving_nucl, None));
                                let id = self.identifier_nucl[&moving_nucl];
                                free_nucl_position.push(self.space_position[&id].into());
                            }
                            prev_doubled = doubled;
                            moving_nucl = moving_nucl.right();
                        }
                    }
                }
            }
        }
        for k in self.identifier_nucl.keys() {
            if !nucl_map.contains_key(k) {
                println!("HO NO :( {:?}", k);
            }
        }
        println!("{:?}", intervals);
        IntervalResult {
            nucl_map,
            helix_map,
            free_nucl_ids,
            free_nucls,
            intervals,
            free_nucl_position,
        }
    }
}

#[derive(Debug)]
pub struct IntervalResult {
    nucl_map: HashMap<Nucl, FreeNucl>,
    helix_map: Vec<usize>,
    free_nucls: Vec<FreeNucl>,
    free_nucl_ids: HashMap<FreeNucl, usize>,
    free_nucl_position: Vec<Vec3>,
    intervals: Vec<(isize, isize)>,
}

enum ShakeTarget {
    FreeNucl(usize),
    Helix(usize),
}

/// Return the length of the shortes line between a point of [a, b] and a poin of [c, d]
fn distance_segment(a: Vec3, b: Vec3, c: Vec3, d: Vec3) -> (f32, Vec3, Vec3, Vec3) {
    let u = b - a;
    let v = d - c;
    let n = u.cross(v);

    if n.mag() < 1e-5 {
        // the segment are almost parallel
        return ((a - c).mag(), (a - c), (a + b) / 2., (c + d) / 2.);
    }

    // lambda u.norm2() - mu u.dot(v) + ((a - c).dot(u)) = 0
    // mu v.norm2() - lambda u.dot(v) + ((c - a).dot(v)) = 0
    let normalise = u.dot(v) / u.mag_sq();

    // mu (v.norm2() - normalise * u.dot(v)) = (-(c - a).dot(v)) - normalise * ((a - c).dot(u))
    let mut mu =
        (-((c - a).dot(v)) - normalise * ((a - c).dot(u))) / (v.mag_sq() - normalise * u.dot(v));

    let mut lambda = (-((a - c).dot(u)) + mu * u.dot(v)) / (u.mag_sq());

    if 0f32 <= mu && mu <= 1f32 && 0f32 <= lambda && lambda <= 1f32 {
        let vec = (a + u * lambda) - (c + v * mu);
        (vec.mag(), vec, a + u * lambda, c + v * mu)
    } else {
        let mut min_dist = std::f32::INFINITY;
        let mut min_vec = Vec3::zero();
        let mut min_point_a = a;
        let mut min_point_c = c;
        lambda = 0f32;
        mu = -((c - a).dot(v)) / v.mag_sq();
        if 0f32 <= mu && mu <= 1f32 {
            let vec = (a + u * lambda) - (c + v * mu);
            if min_dist > vec.mag() {
                min_dist = vec.mag();
                min_vec = vec.clone();
                min_point_a = a + u * lambda;
                min_point_c = c + v * mu;
            }
        } else {
            mu = 0f32;
            let vec = (a + u * lambda) - (c + v * mu);
            if min_dist > vec.mag() {
                min_dist = vec.mag();
                min_vec = vec.clone();
                min_point_a = a + u * lambda;
                min_point_c = c + v * mu;
            }
            mu = 1f32;
            let vec = (a + u * lambda) - (c + v * mu);
            if min_dist > vec.mag() {
                min_dist = vec.mag();
                min_vec = vec.clone();
                min_point_a = a + u * lambda;
                min_point_c = c + v * mu;
            }
        }
        lambda = 1f32;
        mu = (-(c - a).dot(v) + u.dot(v)) / v.mag_sq();
        if 0f32 <= mu && mu <= 1f32 {
            min_dist = min_dist.min(((a + u * lambda) - (c + v * mu)).mag());
            let vec = (a + u * lambda) - (c + v * mu);
            if min_dist > vec.mag() {
                min_dist = vec.mag();
                min_vec = vec.clone();
                min_point_a = a + u * lambda;
                min_point_c = c + v * mu;
            }
        } else {
            mu = 0f32;
            let vec = (a + u * lambda) - (c + v * mu);
            if min_dist > vec.mag() {
                min_dist = vec.mag();
                min_vec = vec.clone();
                min_point_a = a + u * lambda;
                min_point_c = c + v * mu;
            }
            mu = 1f32;
            let vec = (a + u * lambda) - (c + v * mu);
            if min_dist > vec.mag() {
                min_dist = vec.mag();
                min_vec = vec.clone();
                min_point_a = a + u * lambda;
                min_point_c = c + v * mu;
            }
        }
        mu = 0f32;
        lambda = (-((a - c).dot(u)) + mu * u.dot(v)) / (u.mag_sq());
        if 0f32 <= lambda && 1f32 >= lambda {
            let vec = (a + u * lambda) - (c + v * mu);
            if min_dist > vec.mag() {
                min_dist = vec.mag();
                min_vec = vec.clone();
                min_point_a = a + u * lambda;
                min_point_c = c + v * mu;
            }
        } else {
            lambda = 0f32;
            let vec = (a + u * lambda) - (c + v * mu);
            if min_dist > vec.mag() {
                min_dist = vec.mag();
                min_vec = vec.clone();
                min_point_a = a + u * lambda;
                min_point_c = c + v * mu;
            }
            lambda = 1f32;
            let vec = (a + u * lambda) - (c + v * mu);
            if min_dist > vec.mag() {
                min_dist = vec.mag();
                min_vec = vec.clone();
                min_point_a = a + u * lambda;
                min_point_c = c + v * mu;
            }
        }
        mu = 1f32;
        lambda = (-((a - c).dot(u)) + mu * u.dot(v)) / (u.mag_sq());
        if 0f32 <= lambda && 1f32 >= lambda {
            let vec = (a + u * lambda) - (c + v * mu);
            if min_dist > vec.mag() {
                min_dist = vec.mag();
                min_vec = vec.clone();
                min_point_a = a + u * lambda;
                min_point_c = c + v * mu;
            }
        } else {
            lambda = 0f32;
            let vec = (a + u * lambda) - (c + v * mu);
            if min_dist > vec.mag() {
                min_dist = vec.mag();
                min_vec = vec.clone();
                min_point_a = a + u * lambda;
                min_point_c = c + v * mu;
            }
            lambda = 1f32;
            let vec = (a + u * lambda) - (c + v * mu);
            if min_dist > vec.mag() {
                min_dist = vec.mag();
                min_vec = vec.clone();
                min_point_a = a + u * lambda;
                min_point_c = c + v * mu;
            }
        }
        (min_dist, min_vec, min_point_a, min_point_c)
    }
}
