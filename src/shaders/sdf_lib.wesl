struct Surface {
    dist: f32,
    weights: vec4<f32>,
}

fn surface_blend(dist: f32, weights: vec4<f32>) -> Surface {
    var s: Surface;
    s.dist = dist;
    s.weights = weights;
    return s;
}

fn surface(dist: f32, material: u32) -> Surface {
    return surface_blend(dist, vec4(
        select(0.0, 1.0, material == 0u),
        select(0.0, 1.0, material == 1u),
        select(0.0, 1.0, material == 2u),
        select(0.0, 1.0, material >= 3u),
    ));
}

fn sd_sphere(p: vec3<f32>, radius: f32) -> f32 {
    return length(p) - radius;
}

fn sd_box(p: vec3<f32>, half_extents: vec3<f32>) -> f32 {
    let q = abs(p) - half_extents;
    return length(max(q, vec3(0.0))) + min(max(q.x, max(q.y, q.z)), 0.0);
}

fn sd_round_box(p: vec3<f32>, half_extents: vec3<f32>, radius: f32) -> f32 {
    return sd_box(p, half_extents - vec3(radius)) - radius;
}

fn sd_cylinder(p: vec3<f32>, half_height: f32, radius: f32) -> f32 {
    let d = abs(vec2(length(p.xz), p.y)) - vec2(radius, half_height);
    return min(max(d.x, d.y), 0.0) + length(max(d, vec2(0.0)));
}

fn sd_torus(p: vec3<f32>, major_radius: f32, minor_radius: f32) -> f32 {
    let q = vec2(length(p.xz) - major_radius, p.y);
    return length(q) - minor_radius;
}

fn sd_plane(p: vec3<f32>, normal: vec3<f32>, offset: f32) -> f32 {
    return dot(p, normalize(normal)) + offset;
}

fn sd_capsule(p: vec3<f32>, a: vec3<f32>, b: vec3<f32>, radius: f32) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return length(pa - ba * h) - radius;
}

fn op_union(a: Surface, b: Surface) -> Surface {
    if a.dist <= b.dist {
        return a;
    }
    return b;
}

fn op_intersect(a: Surface, b: Surface) -> Surface {
    if a.dist >= b.dist {
        return a;
    }
    return b;
}

fn op_subtract(a: Surface, b: Surface) -> Surface {
    if a.dist >= -b.dist {
        return a;
    }
    return surface_blend(-b.dist, a.weights);
}

fn op_smooth_union(a: Surface, b: Surface, k: f32) -> Surface {
    let h = clamp(0.5 + 0.5 * (b.dist - a.dist) / k, 0.0, 1.0);
    let dist = mix(b.dist, a.dist, h) - k * h * (1.0 - h);
    return surface_blend(dist, mix(b.weights, a.weights, h));
}

fn op_smooth_intersect(a: Surface, b: Surface, k: f32) -> Surface {
    let h = clamp(0.5 - 0.5 * (b.dist - a.dist) / k, 0.0, 1.0);
    let dist = mix(b.dist, a.dist, h) + k * h * (1.0 - h);
    return surface_blend(dist, mix(b.weights, a.weights, h));
}

fn op_smooth_subtract(a: Surface, b: Surface, k: f32) -> Surface {
    let h = clamp(0.5 - 0.5 * (a.dist + b.dist) / k, 0.0, 1.0);
    let dist = mix(a.dist, -b.dist, h) + k * h * (1.0 - h);
    return surface_blend(dist, a.weights);
}

fn op_translate(p: vec3<f32>, offset: vec3<f32>) -> vec3<f32> {
    return p - offset;
}

fn op_repeat(p: vec3<f32>, period: vec3<f32>) -> vec3<f32> {
    return p - period * round(p / period);
}

fn op_rotate_y(p: vec3<f32>, angle: f32) -> vec3<f32> {
    let c = cos(angle);
    let s = sin(angle);
    return vec3(c * p.x - s * p.z, p.y, s * p.x + c * p.z);
}

fn op_rotate_x(p: vec3<f32>, angle: f32) -> vec3<f32> {
    let c = cos(angle);
    let s = sin(angle);
    return vec3(p.x, c * p.y - s * p.z, s * p.y + c * p.z);
}

fn op_rotate_z(p: vec3<f32>, angle: f32) -> vec3<f32> {
    let c = cos(angle);
    let s = sin(angle);
    return vec3(c * p.x - s * p.y, s * p.x + c * p.y, p.z);
}

fn op_mirror_x(p: vec3<f32>) -> vec3<f32> {
    return vec3(abs(p.x), p.y, p.z);
}

fn op_twist_y(p: vec3<f32>, amount: f32) -> vec3<f32> {
    return op_rotate_y(p, amount * p.y);
}
