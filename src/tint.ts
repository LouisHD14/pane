// Adaptive colour: the glass takes the colours of the apps just above the bar, left to right.
// The backend sends 8 colours (one per band) when they change; null turns the tint off.

type Rgb = [number, number, number];

const BANDS = 8;
let on = false;

/** Relative luminance 0..1 (sRGB, close enough for choosing the text colour). */
const luminance = ([r, g, b]: Rgb) => (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255;

export function applyTint(colors: Rgb[] | null) {
  const style = document.body.style;
  if (!colors?.length) {
    if (!on) return;
    on = false;
    document.body.classList.remove("tinted", "light");
    return;
  }
  on = true;
  // The bands are registered colours (style.css), so they glide from one app's colour to the next.
  for (let i = 0; i < BANDS; i++) {
    const [r, g, b] = colors[Math.min(i, colors.length - 1)];
    style.setProperty(`--t${i}`, `rgb(${r} ${g} ${b})`);
  }
  document.body.classList.add("tinted");
  // Light apps (white documents, light mode) make a light bar: switch to dark text there.
  const avg = colors.reduce((s, c) => s + luminance(c), 0) / colors.length;
  document.body.classList.toggle("light", avg > 0.68);
}
