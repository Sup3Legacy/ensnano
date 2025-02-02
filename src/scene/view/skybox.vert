#version 450

layout(location=0) in vec3 a_position;
layout(location=1) in vec2 a_tex_pos;

layout(location=0) out vec3 v_pos;

layout(set=0, binding=0)
uniform Uniforms {
    vec3 u_camera_position;
    mat4 u_view;
    mat4 u_proj;
};

layout(set=1, binding=0) buffer ModelBlock {
    mat4 model_matrix2[];
};

struct Instances {
    float dist;
};

layout(std430, set=2, binding=0) 
buffer InstancesBlock {
    Instances instances[];
};

void main() {
    float dist = instances[0].dist;
    vec4 position = vec4(u_camera_position, 1.) + dist * vec4(a_position, 0.);
    v_pos = a_position;
    gl_Position = u_proj * u_view * position;
}
