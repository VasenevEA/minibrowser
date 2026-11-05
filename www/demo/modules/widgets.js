// Widgets built from other modules: imports of imports.
import label, { fmt } from './format.js';

export class Meter {
  constructor(root, title, kind, min, max) {
    this.kind = kind; this.min = min; this.max = max;
    this.el = document.createElement('div');
    this.el.className = 'meter';
    this.el.innerHTML = '<div class="t">' + title + '</div><div class="v"></div><div class="bar"><i></i></div>';
    root.appendChild(this.el);
    this.v = this.el.querySelector('.v');
    this.bar = this.el.querySelector('i');
  }
  set(v) {
    this.v.textContent = label(this.kind, v);
    const p = Math.max(0, Math.min(1, (v - this.min) / (this.max - this.min)));
    this.bar.style.width = fmt(p * 100, 0).replace(',', '.') + '%';
    this.el.classList.toggle('hi', p > 0.8);
  }
}
