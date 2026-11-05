// Number formatting shared by the widgets.
export const unit = { temp: '°C', pres: 'бар', flow: 'м³/ч' };
export function fmt(v, digits = 1) {
  return v.toFixed(digits).replace('.', ',');
}
export default function label(kind, v) {
  return fmt(v, kind === 'pres' ? 2 : 1) + ' ' + unit[kind];
}
