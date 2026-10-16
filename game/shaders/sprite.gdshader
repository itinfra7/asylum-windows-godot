// Sprite shader for the single MultiMesh that draws every quad of a frame.
// INSTANCE_CUSTOM = atlas rectangle (u, v, w, h) in texels (exact in the 16-bit floats the
// Compatibility renderer packs instance data into); COLOR = instance modulate.
// Sharp-pixel sampling: nearest-neighbour look with bilinear-filtered (anti-aliased) texel
// seams, so fractional scaling stays crisp without shimmering.
shader_type canvas_item;
render_mode blend_premul_alpha, unshaded;

// The fragment COLOR input already contains a texture sample at the unmodified UV, so the
// modulate colour is passed separately.
varying vec4 modulate;

void vertex() {
	UV = (INSTANCE_CUSTOM.xy + UV * INSTANCE_CUSTOM.zw) * TEXTURE_PIXEL_SIZE;
	modulate = COLOR;
}

void fragment() {
	vec2 sz = 1.0 / TEXTURE_PIXEL_SIZE;
	vec2 px = UV * sz;
	vec2 seam = floor(px + 0.5);
	px = seam + clamp((px - seam) / fwidth(px), -0.5, 0.5);
	// The atlas is premultiplied; premultiply the (straight-alpha) modulate colour too.
	COLOR = texture(TEXTURE, px / sz) * vec4(modulate.rgb * modulate.a, modulate.a);
}
