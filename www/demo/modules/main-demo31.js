// The page's entry module: widgets from ./widgets.js, live values on a timer.
import { Meter } from './widgets.js';

const root = document.getElementById('meters');
const meters = [
  new Meter(root, 'Supply temp', 'temp', 40, 95),
  new Meter(root, 'Pressure', 'pres', 0, 6),
  new Meter(root, 'Flow', 'flow', 0, 30),
];
let t = 0;
function step() {
  t += 1;
  meters[0].set(72 + 14 * Math.sin(t / 7));
  meters[1].set(3.2 + 0.8 * Math.cos(t / 5));
  meters[2].set(18 + 9 * Math.sin(t / 3));
}
step();
setInterval(step, 500);
document.getElementById('info').textContent = 'Modules loaded: main-demo31.js → widgets.js → format.js.';
