// Number formatting shared by the widgets.
export const unit = { temp: '°C', pres: 'bar', flow: 'm³/h' };
export function fmt(v, digits = 1) {
  return v.toFixed(digits);
}
export default function label(kind, v) {
  return fmt(v, kind === 'pres' ? 2 : 1) + ' ' + unit[kind];
}
