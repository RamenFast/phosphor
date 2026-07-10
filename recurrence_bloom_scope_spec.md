# Phosphor Scope View Specification: Recurrence Bloom

**Working name:** `recurrence_bloom`  
**Character:** a vectorscope that remembers the music  
**Rendering posture:** scientific instrument underneath; dream machine on top

## One-sentence pitch

Recurrence Bloom turns the current stereo signal into a living harmonic flower, then brings visually similar moments from the recent musical past back as fading phosphor ghosts.

## Design intent

Traditional waveform and spectrum views answer “what is happening now?” MilkDrop-style systems answer “what beautiful reaction can this sound provoke?” Recurrence Bloom adds a third question:

> **Where has the music been before, and how is the present related to it?**

The view should remain useful as a scope. Its geometry is driven by measurable audio properties, not arbitrary scene switching. But it is allowed to be romantic.

## Signal inputs

- Stereo PCM, ideally 48–192 kHz
- Render callback: 60–240 Hz
- Analysis hop: 256–1024 samples
- Rolling semantic memory: 8–32 seconds
- Optional offline memory: full-track feature cache

## Analysis state

| Feature | Suggested method | Visual responsibility |
|---|---|---|
| Stereo carrier | Mid/side or 45° Lissajous transform | Bright central beam |
| Pitch-class energy | Constant-Q chroma, 12 or 24 bins | Petal count, radius, and local curvature |
| Tonal centroid | Six-dimensional Tonnetz projection | Global orientation and slow precession |
| Onset strength | Multi-band spectral flux | Sparks and momentary beam current |
| Rhythmic periodicity | Local tempogram | Breathing rate and rotational lock |
| Stereo width | Side-to-mid energy ratio | Horizontal opening of the bloom |
| Phase coherence | Bandwise inter-channel correlation | Filament tightness and symmetry |
| Spectral brightness | Centroid or reassigned ridge energy | Beam sharpness and halo size |
| Recurrence | Nearest-neighbor similarity over feature history | Returning ghost traces |
| Novelty | Distance from recent feature manifold | Controlled rupture or “new petal” event |

## Geometry

Let `c_k(t)` be normalized chroma energy for pitch class `k`, and let `φ_k` be a stable per-bin phase constant derived from a preset seed.

The harmonic envelope is a closed polar curve:

```text
r(θ,t) = r₀
       + Σₖ [a₀ + a₁ c_k(t)] cos(h_k θ + φ_k + ω_k t)
```

where `h_k` maps pitch classes onto a deliberately aliased family of low integer harmonics. Aliasing here is aesthetic: related notes should produce visibly related petals rather than twelve disconnected spokes.

The projected curve is:

```text
x = r cos(θ + warp) · (1 + stereo_width · W)
y = r sin(θ + warp) · (1 + coherence · C)
```

A conventional stereo carrier is drawn through or over this envelope:

```text
x_c = (L - R) / √2
y_c = (L + R) / √2
```

The result is both readable and theatrical: the carrier tells the truth of the immediate stereo waveform; the bloom tells the truth of its harmonic and temporal context.

## Motif memory

Maintain a feature vector per analysis frame:

```text
z_t = normalize([
  chroma,
  tonnetz,
  band_onsets,
  stereo_width,
  phase_coherence,
  spectral_centroid
])
```

For the current `z_t`, retrieve the strongest non-local matches from the rolling history using cosine affinity or a sparse recurrence matrix.

Render the top matches as historical beam paths:

- newest match: sharp, close to the current radius
- older match: slightly rotated, dimmer, more diffuse
- repeated match cluster: stable “memory petals”
- structurally new passage: ghosts recede; the live trace gains space

This makes verse/chorus returns, repeated riffs, and harmonic callbacks visible without a timeline or labels.

## CRT model

Use two independent decay reservoirs:

```text
fast(t + Δt) = fast(t) · exp(-Δt / τ_fast) + beam
slow(t + Δt) = slow(t) · exp(-Δt / τ_slow) + beam · slow_gain
output = fast + slow
```

Suggested defaults:

- `τ_fast = 55 ms`
- `τ_slow = 1.8 s`
- bloom radius driven by beam current
- scanlines applied after phosphor integration
- optional aperture mask at high zoom
- subtle glass reflection only; never obscure the trace
- decay must be time-based, not frame-based

## Interaction

Minimal controls:

- **Memory:** 0–32 s
- **Bloom:** harmonic geometry amount
- **Carrier:** raw stereo trace amount
- **Recurrence:** number and strength of ghosts
- **Tempo Lock:** free / half / beat / bar
- **Novelty Flash:** off / restrained / theatrical
- **Phosphor:** P7 green / amber / ice / custom
- **CRT:** clean / studio / consumer / exhausted tube

## Preset personalities

- **Instrument:** carrier-forward, no particles, accurate graticule
- **Night Garden:** larger harmonic bloom, long memory, gentle sparks
- **Windows 2001:** high motion, bolder modulation, automatic scene energy
- **Deep Listening:** slow rotation, 32-second recurrence, minimal flashes
- **Broadcast Ghost:** amber phosphor, narrow bandwidth, visible retrace texture

## Performance budget

At 1080p/120 Hz:

- DSP features: under 1.5 ms per analysis hop on a modern desktop CPU
- Geometry generation: under 0.5 ms
- GPU beam deposition + dual decay + bloom: under 3 ms
- No allocation or locking in the audio callback
- Feature history stored in fixed-capacity rings
- Recurrence search may run asynchronously against copied feature frames
- Degrade gracefully by reducing ghost count, curve samples, then bloom taps

## Validation

The view should pass both artistic and engineering tests:

1. Mono collapses toward a narrow, symmetric carrier.
2. Polarity inversion visibly reverses the appropriate stereo axis.
3. A static sine pair produces a stable, reproducible figure.
4. Chord changes alter petal geometry without random scene cuts.
5. Repeated sections restore recognizably similar ghost structures.
6. Silence decays according to wall-clock time at every FPS cap.
7. The same seeded input produces bitwise-identical geometry before rasterization.
8. Agent-visible state reports every control, feature value, and active preset.

## Why it belongs in Phosphor

Phosphor already has the bones of an instrument: waveform-derived geometry, beam deposition, persistence, themes, and live audio routing. Recurrence Bloom would let it grow toward the emotional territory of classic media-player visualizers without becoming a disconnected shader jukebox.

It would be a visualizer with memory—and a scope with a soul.
