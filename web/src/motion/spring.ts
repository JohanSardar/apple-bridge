export type Spring = { response: number; damping: number };
export type SpringState = { value: number; velocity: number };
export function sampleSpring(from: number, target: number, velocity: number, elapsed: number, spring: Spring): SpringState {
  const omega = 2 * Math.PI / Math.max(.001, spring.response);
  const zeta = Math.max(0, spring.damping);
  const x = from - target;
  const t = Math.max(0, elapsed);
  if (Math.abs(zeta - 1) < .0001) {
    const b = velocity + omega * x;
    const decay = Math.exp(-omega * t);
    return { value: target + (x + b * t) * decay, velocity: (b - omega * (x + b * t)) * decay };
  }
  if (zeta < 1) {
    const damped = omega * Math.sqrt(1 - zeta * zeta);
    const b = (velocity + zeta * omega * x) / damped;
    const decay = Math.exp(-zeta * omega * t);
    const sin = Math.sin(damped * t), cos = Math.cos(damped * t);
    const position = x * cos + b * sin;
    return { value: target + position * decay, velocity: decay * (-zeta * omega * position + damped * (-x * sin + b * cos)) };
  }
  const root = Math.sqrt(zeta * zeta - 1);
  const r1 = -omega * (zeta - root), r2 = -omega * (zeta + root);
  const a = (velocity - r2 * x) / (r1 - r2), b = x - a;
  return { value: target + a * Math.exp(r1*t) + b * Math.exp(r2*t), velocity: a*r1*Math.exp(r1*t) + b*r2*Math.exp(r2*t) };
}
export function rubberBand(distance: number, dimension: number, constant = .55) {
  return Math.sign(distance) * (1 - 1 / (Math.abs(distance) * constant / Math.max(1,dimension) + 1)) * dimension;
}
