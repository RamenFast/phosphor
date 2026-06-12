"""GPU (OpenGL) renderer: a real CRT beam simulation in shaders.

No PyOpenGL needed — GTK already links libepoxy, the GL dispatch library,
so we bind the handful of GL functions we use directly with ctypes.

How it works, per frame:
  1. decay pass   — the RG16F "energy" texture (R = P7 flash energy,
                    G = P7 glow energy) is multiplied by per-layer decay
                    factors into a second texture (ping-pong).
  2. beam pass    — every segment becomes one instanced quad; the fragment
                    shader computes the analytic distance to the segment and
                    deposits a Gaussian beam cross-section, blended additively.
                    This is the woscope-style trick that makes oscilloscope
                    music look like a real scope.
  3. composite    — energy is tone-mapped (1 - e^-kx, like phosphor
                    saturation) and tinted: flash energy in blue-white,
                    glow energy in the theme beam color, plus a procedural
                    graticule, straight into GtkGLArea's framebuffer.
"""

import ctypes
from array import array

import gi

gi.require_version("Gtk", "3.0")
from gi.repository import Gtk, GLib  # noqa: E402

# ---------------------------------------------------------------------------
# Minimal GL binding via libepoxy
# ---------------------------------------------------------------------------

GL_COLOR_BUFFER_BIT = 0x4000
GL_TRIANGLES = 0x0004
GL_FLOAT = 0x1406
GL_ARRAY_BUFFER = 0x8892
GL_STREAM_DRAW = 0x88E0
GL_VERTEX_SHADER = 0x8B31
GL_FRAGMENT_SHADER = 0x8B30
GL_COMPILE_STATUS = 0x8B81
GL_LINK_STATUS = 0x8B82
GL_TEXTURE_2D = 0x0DE1
GL_TEXTURE0 = 0x84C0
GL_TEXTURE_MIN_FILTER = 0x2801
GL_TEXTURE_MAG_FILTER = 0x2800
GL_TEXTURE_WRAP_S = 0x2802
GL_TEXTURE_WRAP_T = 0x2803
GL_NEAREST = 0x2600
GL_LINEAR = 0x2601
GL_CLAMP_TO_EDGE = 0x812F
GL_FRAMEBUFFER = 0x8D40
GL_COLOR_ATTACHMENT0 = 0x8CE0
GL_FRAMEBUFFER_COMPLETE = 0x8CD5
GL_FRAMEBUFFER_BINDING = 0x8CA6
GL_BLEND = 0x0BE2
GL_ONE = 1
GL_RG = 0x8227
GL_RG16F = 0x822F


class GLFunctions:
    """The ~30 GL entry points we need, resolved from libepoxy."""

    _SIGNATURES = {
        "glCreateShader": (ctypes.c_uint, [ctypes.c_uint]),
        "glShaderSource": (None, [ctypes.c_uint, ctypes.c_int,
                                  ctypes.POINTER(ctypes.c_char_p),
                                  ctypes.POINTER(ctypes.c_int)]),
        "glCompileShader": (None, [ctypes.c_uint]),
        "glGetShaderiv": (None, [ctypes.c_uint, ctypes.c_uint,
                                 ctypes.POINTER(ctypes.c_int)]),
        "glGetShaderInfoLog": (None, [ctypes.c_uint, ctypes.c_int,
                                      ctypes.POINTER(ctypes.c_int), ctypes.c_char_p]),
        "glCreateProgram": (ctypes.c_uint, []),
        "glAttachShader": (None, [ctypes.c_uint, ctypes.c_uint]),
        "glLinkProgram": (None, [ctypes.c_uint]),
        "glGetProgramiv": (None, [ctypes.c_uint, ctypes.c_uint,
                                  ctypes.POINTER(ctypes.c_int)]),
        "glGetProgramInfoLog": (None, [ctypes.c_uint, ctypes.c_int,
                                       ctypes.POINTER(ctypes.c_int), ctypes.c_char_p]),
        "glDeleteShader": (None, [ctypes.c_uint]),
        "glUseProgram": (None, [ctypes.c_uint]),
        "glGetUniformLocation": (ctypes.c_int, [ctypes.c_uint, ctypes.c_char_p]),
        "glUniform1i": (None, [ctypes.c_int, ctypes.c_int]),
        "glUniform1f": (None, [ctypes.c_int, ctypes.c_float]),
        "glUniform2f": (None, [ctypes.c_int, ctypes.c_float, ctypes.c_float]),
        "glUniform3f": (None, [ctypes.c_int, ctypes.c_float, ctypes.c_float, ctypes.c_float]),
        "glGenVertexArrays": (None, [ctypes.c_int, ctypes.POINTER(ctypes.c_uint)]),
        "glBindVertexArray": (None, [ctypes.c_uint]),
        "glGenBuffers": (None, [ctypes.c_int, ctypes.POINTER(ctypes.c_uint)]),
        "glBindBuffer": (None, [ctypes.c_uint, ctypes.c_uint]),
        "glBufferData": (None, [ctypes.c_uint, ctypes.c_ssize_t,
                                ctypes.c_void_p, ctypes.c_uint]),
        "glEnableVertexAttribArray": (None, [ctypes.c_uint]),
        "glVertexAttribPointer": (None, [ctypes.c_uint, ctypes.c_int, ctypes.c_uint,
                                         ctypes.c_ubyte, ctypes.c_int, ctypes.c_void_p]),
        "glVertexAttribDivisor": (None, [ctypes.c_uint, ctypes.c_uint]),
        "glDrawArrays": (None, [ctypes.c_uint, ctypes.c_int, ctypes.c_int]),
        "glDrawArraysInstanced": (None, [ctypes.c_uint, ctypes.c_int,
                                         ctypes.c_int, ctypes.c_int]),
        "glGenTextures": (None, [ctypes.c_int, ctypes.POINTER(ctypes.c_uint)]),
        "glBindTexture": (None, [ctypes.c_uint, ctypes.c_uint]),
        "glActiveTexture": (None, [ctypes.c_uint]),
        "glTexImage2D": (None, [ctypes.c_uint, ctypes.c_int, ctypes.c_int,
                                ctypes.c_int, ctypes.c_int, ctypes.c_int,
                                ctypes.c_uint, ctypes.c_uint, ctypes.c_void_p]),
        "glTexParameteri": (None, [ctypes.c_uint, ctypes.c_uint, ctypes.c_int]),
        "glDeleteTextures": (None, [ctypes.c_int, ctypes.POINTER(ctypes.c_uint)]),
        "glGenFramebuffers": (None, [ctypes.c_int, ctypes.POINTER(ctypes.c_uint)]),
        "glBindFramebuffer": (None, [ctypes.c_uint, ctypes.c_uint]),
        "glFramebufferTexture2D": (None, [ctypes.c_uint, ctypes.c_uint, ctypes.c_uint,
                                          ctypes.c_uint, ctypes.c_int]),
        "glCheckFramebufferStatus": (ctypes.c_uint, [ctypes.c_uint]),
        "glDeleteFramebuffers": (None, [ctypes.c_int, ctypes.POINTER(ctypes.c_uint)]),
        "glGetIntegerv": (None, [ctypes.c_uint, ctypes.POINTER(ctypes.c_int)]),
        "glViewport": (None, [ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_int]),
        "glClearColor": (None, [ctypes.c_float, ctypes.c_float,
                                ctypes.c_float, ctypes.c_float]),
        "glClear": (None, [ctypes.c_uint]),
        "glEnable": (None, [ctypes.c_uint]),
        "glDisable": (None, [ctypes.c_uint]),
        "glBlendFunc": (None, [ctypes.c_uint, ctypes.c_uint]),
        "glGetError": (ctypes.c_uint, []),
    }

    def __init__(self):
        # epoxy exports each entry point as a data symbol: a function pointer
        # named epoxy_glFoo whose stub resolves against the current GL context
        # on call — exactly what we want under GtkGLArea.
        library = ctypes.CDLL("libepoxy.so.0")
        for name, (restype, argtypes) in self._SIGNATURES.items():
            pointer = ctypes.c_void_p.in_dll(library, f"epoxy_{name}")
            prototype = ctypes.CFUNCTYPE(restype, *argtypes)
            setattr(self, name, prototype(pointer.value))


try:
    gl = GLFunctions()
    GL_BINDINGS_AVAILABLE = True
except (OSError, AttributeError):
    gl = None
    GL_BINDINGS_AVAILABLE = False


# ---------------------------------------------------------------------------
# Shaders
# ---------------------------------------------------------------------------

FULLSCREEN_VERTEX_SHADER = b"""
#version 330 core
out vec2 uv;
void main() {
    vec2 corner = vec2((gl_VertexID << 1) & 2, gl_VertexID & 2);
    uv = corner;
    gl_Position = vec4(corner * 2.0 - 1.0, 0.0, 1.0);
}
"""

DECAY_FRAGMENT_SHADER = b"""
#version 330 core
in vec2 uv;
out vec4 out_energy;
uniform sampler2D energy_texture;
uniform vec2 decay_keep;   // (flash keep, glow keep) per frame
void main() {
    vec2 energy = texture(energy_texture, uv).rg * decay_keep;
    energy = max(energy - 0.0004, 0.0);   // let faint trails truly reach zero
    out_energy = vec4(energy, 0.0, 1.0);
}
"""

BEAM_VERTEX_SHADER = b"""
#version 330 core
layout(location = 0) in vec4 segment;     // p0.xy, p1.xy in pixels (per instance)
layout(location = 1) in float intensity;  // per instance
uniform vec2 viewport_size;
uniform float beam_radius;
flat out vec2 segment_p0;
flat out vec2 segment_p1;
flat out float segment_intensity;
const vec2 corner_table[6] = vec2[6](
    vec2(0., -1.), vec2(1., -1.), vec2(1., 1.),
    vec2(0., -1.), vec2(1., 1.), vec2(0., 1.));
void main() {
    vec2 p0 = segment.xy, p1 = segment.zw;
    vec2 direction = p1 - p0;
    float segment_length = length(direction);
    vec2 tangent = segment_length > 1e-4 ? direction / segment_length : vec2(1., 0.);
    vec2 normal = vec2(-tangent.y, tangent.x);
    vec2 corner = corner_table[gl_VertexID];
    vec2 position = mix(p0 - tangent * beam_radius, p1 + tangent * beam_radius, corner.x)
                  + normal * beam_radius * corner.y;
    segment_p0 = p0;
    segment_p1 = p1;
    segment_intensity = intensity;
    vec2 ndc = position / viewport_size * 2.0 - 1.0;
    gl_Position = vec4(ndc.x, -ndc.y, 0.0, 1.0);
}
"""

BEAM_FRAGMENT_SHADER = b"""
#version 330 core
flat in vec2 segment_p0;
flat in vec2 segment_p1;
flat in float segment_intensity;
uniform vec2 viewport_size;
uniform float beam_sigma;
out vec4 out_energy;
void main() {
    vec2 pixel = vec2(gl_FragCoord.x, viewport_size.y - gl_FragCoord.y);
    vec2 to_pixel = pixel - segment_p0;
    vec2 along = segment_p1 - segment_p0;
    float t = clamp(dot(to_pixel, along) / max(dot(along, along), 1e-6), 0.0, 1.0);
    float distance_to_beam = length(to_pixel - along * t);
    float falloff = exp(-distance_to_beam * distance_to_beam
                        / (2.0 * beam_sigma * beam_sigma));
    float energy = segment_intensity * falloff;
    out_energy = vec4(energy, energy * 0.85, 0.0, 1.0);
}
"""

COMPOSITE_FRAGMENT_SHADER = b"""
#version 330 core
in vec2 uv;
out vec4 out_color;
uniform sampler2D energy_texture;
uniform vec2 viewport_size;
uniform vec3 beam_color;
uniform vec3 flash_color;
uniform vec3 grid_color;
uniform vec3 background_color;
uniform float grid_enabled;
uniform float grid_spacing;   // device pixels per graticule division

float grid_line(float coordinate, float spacing) {
    float distance_to_line = abs(coordinate - spacing * floor(coordinate / spacing + 0.5));
    return 1.0 - smoothstep(0.4, 1.0, distance_to_line);
}

void main() {
    vec2 energy = texture(energy_texture, uv).rg;
    float flash = 1.0 - exp(-1.7 * energy.r);   // phosphor saturation curve
    float glow  = 1.0 - exp(-1.7 * energy.g);

    vec3 color = background_color;
    if (grid_enabled > 0.5) {
        // grid is centered so divisions track the beam's amplitude scale (zoom)
        vec2 from_center = uv * viewport_size - viewport_size * 0.5;
        float minor = max(grid_line(from_center.x, grid_spacing),
                          grid_line(from_center.y, grid_spacing));
        float axis = max(1.0 - smoothstep(0.5, 1.2, abs(from_center.x)),
                         1.0 - smoothstep(0.5, 1.2, abs(from_center.y)));
        color += grid_color * (minor * 0.07 + axis * 0.10);
    }
    color += beam_color * glow + flash_color * flash * 0.8;
    out_color = vec4(color, 1.0);
}
"""

FLASH_KEEP_PER_FRAME = 0.50


def _compile_program(vertex_source, fragment_source):
    def compile_shader(kind, source):
        shader = gl.glCreateShader(kind)
        source_array = (ctypes.c_char_p * 1)(source)
        length_array = (ctypes.c_int * 1)(len(source))
        gl.glShaderSource(shader, 1, source_array, length_array)
        gl.glCompileShader(shader)
        status = ctypes.c_int(0)
        gl.glGetShaderiv(shader, GL_COMPILE_STATUS, ctypes.byref(status))
        if not status.value:
            log = ctypes.create_string_buffer(4096)
            gl.glGetShaderInfoLog(shader, 4096, None, log)
            raise RuntimeError(f"shader compile failed: {log.value.decode()}")
        return shader

    vertex_shader = compile_shader(GL_VERTEX_SHADER, vertex_source)
    fragment_shader = compile_shader(GL_FRAGMENT_SHADER, fragment_source)
    program = gl.glCreateProgram()
    gl.glAttachShader(program, vertex_shader)
    gl.glAttachShader(program, fragment_shader)
    gl.glLinkProgram(program)
    status = ctypes.c_int(0)
    gl.glGetProgramiv(program, GL_LINK_STATUS, ctypes.byref(status))
    if not status.value:
        log = ctypes.create_string_buffer(4096)
        gl.glGetProgramInfoLog(program, 4096, None, log)
        raise RuntimeError(f"program link failed: {log.value.decode()}")
    gl.glDeleteShader(vertex_shader)
    gl.glDeleteShader(fragment_shader)
    return program


def _uniforms(program, names):
    return {name: gl.glGetUniformLocation(program, name.encode()) for name in names}


class GLBeamRenderer(Gtk.GLArea):
    """GtkGLArea that runs the decay → beam → composite pipeline."""

    def __init__(self, on_failure):
        super().__init__()
        self.on_failure = on_failure       # called once if GL can't initialize
        self.theme = None
        self.persistence = 0.7
        self.grid_enabled = True
        self.grid_spacing_fraction = 0.1125  # of min(viewport); tracks gain/zoom
        self.beam_focus = 1.6              # beam sigma in logical pixels
        self.supersample = 1               # energy buffer scale: 1 = native, 2 = fine
        self.pending_segments = None       # set by advance(), consumed in render
        self.ready = False
        self._failed = False

        self.set_required_version(3, 3)
        self.set_has_depth_buffer(False)
        self.set_has_stencil_buffer(False)
        self.connect("realize", self._on_realize)
        self.connect("render", self._on_render)

    def advance(self, segments):
        self.pending_segments = segments
        self.queue_render()

    # -- GL lifecycle --------------------------------------------------------

    def _fail(self, message):
        if not self._failed:
            self._failed = True
            GLib.idle_add(self.on_failure, message)

    def _on_realize(self, area):
        area.make_current()
        if area.get_error() is not None:
            self._fail(str(area.get_error()))
            return
        try:
            self.decay_program = _compile_program(FULLSCREEN_VERTEX_SHADER,
                                                  DECAY_FRAGMENT_SHADER)
            self.decay_uniforms = _uniforms(self.decay_program,
                                            ["energy_texture", "decay_keep"])
            self.beam_program = _compile_program(BEAM_VERTEX_SHADER,
                                                 BEAM_FRAGMENT_SHADER)
            self.beam_uniforms = _uniforms(self.beam_program,
                                           ["viewport_size", "beam_radius", "beam_sigma"])
            self.composite_program = _compile_program(FULLSCREEN_VERTEX_SHADER,
                                                      COMPOSITE_FRAGMENT_SHADER)
            self.composite_uniforms = _uniforms(self.composite_program, [
                "energy_texture", "viewport_size", "beam_color", "flash_color",
                "grid_color", "background_color", "grid_enabled", "grid_spacing"])

            vertex_arrays = (ctypes.c_uint * 2)()
            gl.glGenVertexArrays(2, vertex_arrays)
            self.fullscreen_vao, self.beam_vao = vertex_arrays[0], vertex_arrays[1]

            buffers = (ctypes.c_uint * 1)()
            gl.glGenBuffers(1, buffers)
            self.instance_buffer = buffers[0]
            gl.glBindVertexArray(self.beam_vao)
            gl.glBindBuffer(GL_ARRAY_BUFFER, self.instance_buffer)
            stride = 5 * 4  # p0.xy, p1.xy, intensity as float32
            gl.glEnableVertexAttribArray(0)
            gl.glVertexAttribPointer(0, 4, GL_FLOAT, 0, stride, ctypes.c_void_p(0))
            gl.glVertexAttribDivisor(0, 1)
            gl.glEnableVertexAttribArray(1)
            gl.glVertexAttribPointer(1, 1, GL_FLOAT, 0, stride, ctypes.c_void_p(16))
            gl.glVertexAttribDivisor(1, 1)
            gl.glBindVertexArray(0)

            self.energy_textures = None
            self.energy_framebuffers = None
            self.texture_width = 0
            self.texture_height = 0
            self.current_texture_index = 0
            self.ready = True
        except RuntimeError as error:
            self._fail(str(error))

    def _ensure_energy_textures(self, width, height):
        if (self.energy_textures is not None
                and width == self.texture_width and height == self.texture_height):
            return
        if self.energy_textures is not None:
            gl.glDeleteFramebuffers(2, self.energy_framebuffers)
            gl.glDeleteTextures(2, self.energy_textures)
        self.texture_width, self.texture_height = width, height
        self.energy_textures = (ctypes.c_uint * 2)()
        self.energy_framebuffers = (ctypes.c_uint * 2)()
        gl.glGenTextures(2, self.energy_textures)
        gl.glGenFramebuffers(2, self.energy_framebuffers)
        for index in range(2):
            gl.glBindTexture(GL_TEXTURE_2D, self.energy_textures[index])
            gl.glTexImage2D(GL_TEXTURE_2D, 0, GL_RG16F, width, height, 0,
                            GL_RG, GL_FLOAT, None)
            # linear so a supersampled energy buffer downfilters smoothly
            for parameter, value in ((GL_TEXTURE_MIN_FILTER, GL_LINEAR),
                                     (GL_TEXTURE_MAG_FILTER, GL_LINEAR),
                                     (GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE),
                                     (GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE)):
                gl.glTexParameteri(GL_TEXTURE_2D, parameter, value)
            gl.glBindFramebuffer(GL_FRAMEBUFFER, self.energy_framebuffers[index])
            gl.glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0,
                                      GL_TEXTURE_2D, self.energy_textures[index], 0)
            gl.glClearColor(0, 0, 0, 0)
            gl.glClear(GL_COLOR_BUFFER_BIT)

    # -- per-frame pipeline ----------------------------------------------------

    def _on_render(self, area, _gl_context):
        if not self.ready or self.theme is None:
            return True
        scale = self.get_scale_factor()
        width = self.get_allocated_width() * scale
        height = self.get_allocated_height() * scale
        if width < 2 or height < 2:
            return True

        gtk_framebuffer = ctypes.c_int(0)
        gl.glGetIntegerv(GL_FRAMEBUFFER_BINDING, ctypes.byref(gtk_framebuffer))

        supersample = max(1, int(self.supersample))
        texture_width, texture_height = width * supersample, height * supersample
        self._ensure_energy_textures(texture_width, texture_height)
        if self.pending_segments is not None:
            segments, self.pending_segments = self.pending_segments, None
            self._decay_pass(texture_width, texture_height)
            if segments:
                self._beam_pass(segments, texture_width, texture_height,
                                scale * supersample)

        self._composite_pass(gtk_framebuffer.value, width, height)
        return True

    def _decay_pass(self, width, height):
        source = self.current_texture_index
        target = 1 - source
        gl.glBindFramebuffer(GL_FRAMEBUFFER, self.energy_framebuffers[target])
        gl.glViewport(0, 0, width, height)
        gl.glDisable(GL_BLEND)
        gl.glUseProgram(self.decay_program)
        gl.glActiveTexture(GL_TEXTURE0)
        gl.glBindTexture(GL_TEXTURE_2D, self.energy_textures[source])
        gl.glUniform1i(self.decay_uniforms["energy_texture"], 0)
        glow_keep = 1.0 - max(0.02, (1.0 - self.persistence) * 0.6)
        gl.glUniform2f(self.decay_uniforms["decay_keep"], FLASH_KEEP_PER_FRAME, glow_keep)
        gl.glBindVertexArray(self.fullscreen_vao)
        gl.glDrawArrays(GL_TRIANGLES, 0, 3)
        self.current_texture_index = target

    def _beam_pass(self, segments, width, height, pixel_scale):
        instance_data = array("f")
        for segment in segments:
            instance_data.extend(segment)
        raw = instance_data.tobytes()
        if pixel_scale != 1:
            # segment coordinates are in logical pixels; scale to buffer pixels
            scaled = array("f", instance_data)
            for index in range(0, len(scaled), 5):
                for offset in range(4):
                    scaled[index + offset] *= pixel_scale
            raw = scaled.tobytes()

        beam_sigma = max(0.4, self.beam_focus) * pixel_scale
        gl.glBindFramebuffer(GL_FRAMEBUFFER,
                             self.energy_framebuffers[self.current_texture_index])
        gl.glViewport(0, 0, width, height)
        gl.glEnable(GL_BLEND)
        gl.glBlendFunc(GL_ONE, GL_ONE)
        gl.glUseProgram(self.beam_program)
        gl.glUniform2f(self.beam_uniforms["viewport_size"], width, height)
        gl.glUniform1f(self.beam_uniforms["beam_radius"], beam_sigma * 4.0)
        gl.glUniform1f(self.beam_uniforms["beam_sigma"], beam_sigma)
        gl.glBindVertexArray(self.beam_vao)
        gl.glBindBuffer(GL_ARRAY_BUFFER, self.instance_buffer)
        gl.glBufferData(GL_ARRAY_BUFFER, len(raw), raw, GL_STREAM_DRAW)
        gl.glDrawArraysInstanced(GL_TRIANGLES, 0, 6, len(segments))
        gl.glDisable(GL_BLEND)

    def _composite_pass(self, gtk_framebuffer, width, height):
        gl.glBindFramebuffer(GL_FRAMEBUFFER, gtk_framebuffer)
        gl.glViewport(0, 0, width, height)
        gl.glDisable(GL_BLEND)
        gl.glUseProgram(self.composite_program)
        gl.glActiveTexture(GL_TEXTURE0)
        gl.glBindTexture(GL_TEXTURE_2D, self.energy_textures[self.current_texture_index])
        uniforms = self.composite_uniforms
        gl.glUniform1i(uniforms["energy_texture"], 0)
        gl.glUniform2f(uniforms["viewport_size"], width, height)
        gl.glUniform3f(uniforms["beam_color"], *self.theme.beam_color)
        gl.glUniform3f(uniforms["flash_color"], *self.theme.flash_color)
        gl.glUniform3f(uniforms["grid_color"], *self.theme.grid_color)
        gl.glUniform3f(uniforms["background_color"], *self.theme.background_color)
        gl.glUniform1f(uniforms["grid_enabled"], 1.0 if self.grid_enabled else 0.0)
        gl.glUniform1f(uniforms["grid_spacing"],
                       self.grid_spacing_fraction * min(width, height))
        gl.glBindVertexArray(self.fullscreen_vao)
        gl.glDrawArrays(GL_TRIANGLES, 0, 3)
        gl.glBindVertexArray(0)
