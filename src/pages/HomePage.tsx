import { useState, useEffect, useRef, useMemo } from 'react';

// ===== Logo 组件 =====
const Logo = ({ style, imgStyle }: { style?: React.CSSProperties; imgStyle?: React.CSSProperties }) => (
  <div style={{ display: 'inline-block', ...style }}>
    <img src="/logo.png" alt="PurinBox"
      style={{ maxWidth: 280, width: '100%', height: 'auto', objectFit: 'contain', userSelect: 'none', pointerEvents: 'none', ...imgStyle }}
      draggable={false} />
  </div>
);

// ===== 16 种搞怪动画 =====

/** 1. 鸡块旋转 */
function Spin() {
  return <Logo style={{ animation: 'anim-spin 1.2s linear infinite' }} />;
}

/** 2. DVD 弹弹乐 — 整页弹来弹去，碰壁果冻形变 */
function DvdBounce() {
  const containerRef = useRef<HTMLDivElement>(null);
  const logoRef = useRef<HTMLDivElement>(null);
  const pos = useRef({ x: 60, y: 40 });
  const vel = useRef({ vx: 2.2, vy: 1.6 });
  const hue = useRef(0);
  const squash = useRef({ sx: 1, sy: 1, decay: 0 });
  const initialized = useRef(false);
  const [s, setS] = useState<React.CSSProperties>({
    position: 'absolute',
    left: '50%', top: '50%',
    transform: 'translate(-50%, -50%)',
  });

  useEffect(() => {
    let raf: number;
    const tick = () => {
      const c = containerRef.current, l = logoRef.current;
      if (!c || !l) { raf = requestAnimationFrame(tick); return; }
      const cw = c.clientWidth, ch = c.clientHeight, lw = l.clientWidth, lh = l.clientHeight;
      const p = pos.current, v = vel.current, sq = squash.current;

      // 首帧居中
      if (!initialized.current) {
        p.x = (cw - lw) / 2;
        p.y = (ch - lh) / 2;
        initialized.current = true;
      }

      p.x += v.vx; p.y += v.vy;

      let hitX = false, hitY = false;
      if (p.x <= 0) { v.vx = Math.abs(v.vx); p.x = 0; hitX = true; }
      if (p.x + lw >= cw) { v.vx = -Math.abs(v.vx); p.x = cw - lw; hitX = true; }
      if (p.y <= 0) { v.vy = Math.abs(v.vy); p.y = 0; hitY = true; }
      if (p.y + lh >= ch) { v.vy = -Math.abs(v.vy); p.y = ch - lh; hitY = true; }

      if (hitX) { sq.sx = 0.75; sq.sy = 1.25; sq.decay = 1; hue.current += 50; }
      if (hitY) { sq.sx = 1.25; sq.sy = 0.75; sq.decay = 1; hue.current += 50; }

      if (sq.decay > 0.01) {
        sq.decay *= 0.88;
        sq.sx = 1 + (sq.sx - 1) * sq.decay;
        sq.sy = 1 + (sq.sy - 1) * sq.decay;
      } else { sq.sx = 1; sq.sy = 1; }

      setS({
        position: 'absolute', left: p.x, top: p.y,
        transform: `scale(${sq.sx}, ${sq.sy})`,
        filter: `hue-rotate(${hue.current}deg)`,
        transition: 'filter 0.4s',
      });
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, []);

  return (
    <div ref={containerRef} style={{ position: 'absolute', inset: 0, overflow: 'hidden' }}>
      <div ref={logoRef} style={{ display: 'inline-block', ...s }}>
        <Logo />
      </div>
    </div>
  );
}

/** 3. 果冻抖动 */
function Jelly() {
  return <Logo style={{ animation: 'anim-jelly 1.5s ease-in-out infinite' }} />;
}

/** 4. 醉酒摇摆 */
function Drunk() {
  return <Logo style={{ animation: 'anim-drunk 2.5s ease-in-out infinite' }} />;
}

/** 5. 心跳膨胀 */
function Heartbeat() {
  return <Logo style={{ animation: 'anim-heartbeat 1s ease-in-out infinite' }} />;
}

/** 6. 火箭蹦跳 */
function Rocket() {
  return <Logo style={{ animation: 'anim-rocket 1.8s cubic-bezier(0.34, 1.56, 0.64, 1) infinite' }} />;
}

/** 7. 摇滚摇摆 — 左右有节奏摇 */
function RockSwing() {
  return <Logo style={{ animation: 'anim-rock 0.6s ease-in-out infinite alternate', transformOrigin: 'center bottom' }} />;
}

/** 8. 海浪漂浮 */
function WaveFloat() {
  return <Logo style={{ animation: 'anim-wave 3s ease-in-out infinite' }} />;
}

/** 9. 翻跟斗 — 不定期翻一圈 */
function Somersault() {
  return <Logo style={{ animation: 'anim-flip 2s ease-in-out infinite' }} />;
}

/** 10. 幽灵闪现 */
function GhostFade() {
  return <Logo style={{ animation: 'anim-ghost 2.5s ease-in-out infinite' }} />;
}

/** 11. 龙卷风 — 旋转 + 缩放 + 左右飘 */
function Tornado() {
  return <Logo style={{ animation: 'anim-tornado 2s linear infinite' }} />;
}

/** 12. 弹簧弹跳 — 压扁拉长 */
function SpringBounce() {
  return <Logo style={{ animation: 'anim-spring 1s ease-in-out infinite' }} />;
}

/** 13. 迪斯科 */
function Disco() {
  return <Logo style={{ animation: 'anim-disco 0.8s steps(8) infinite' }} imgStyle={{ filter: 'saturate(2)' }} />;
}

/** 14. 毛毛虫蠕动 */
function Caterpillar() {
  return <Logo style={{ animation: 'anim-worm 1.5s ease-in-out infinite' }} />;
}

/** 15. 钟摆摇 */
function Pendulum() {
  return <Logo style={{ animation: 'anim-pendulum 2s ease-in-out infinite', transformOrigin: 'center top' }} />;
}

/** 16. 像素抖动（怀旧游戏感） */
function PixelShake() {
  return <Logo style={{ animation: 'anim-pixel 0.1s steps(2) infinite' }} imgStyle={{ imageRendering: 'pixelated' }} />;
}

// ===== 动画列表 =====
const ANIMATIONS = [
  Spin, DvdBounce, Jelly, Drunk, Heartbeat, Rocket,
  RockSwing, WaveFloat, Somersault, GhostFade, Tornado, SpringBounce,
  Disco, Caterpillar, Pendulum, PixelShake,
] as const;

// ===== CSS Keyframes =====
const KEYFRAMES = `
@keyframes anim-spin {
  0% { transform: rotate(0deg); }
  100% { transform: rotate(360deg); }
}
@keyframes anim-jelly {
  0%, 100% { transform: scale(1, 1); }
  15% { transform: scale(1.18, 0.82); }
  30% { transform: scale(0.82, 1.18); }
  45% { transform: scale(1.1, 0.9); }
  60% { transform: scale(0.95, 1.05); }
  75% { transform: scale(1.03, 0.97); }
}
@keyframes anim-drunk {
  0% { transform: rotate(0deg) translateX(0); }
  10% { transform: rotate(8deg) translateX(20px); }
  20% { transform: rotate(-6deg) translateX(-15px) translateY(-10px); }
  30% { transform: rotate(10deg) translateX(25px) translateY(5px); }
  40% { transform: rotate(-8deg) translateX(-20px) translateY(-8px); }
  50% { transform: rotate(5deg) translateX(10px) translateY(12px); }
  60% { transform: rotate(-10deg) translateX(-25px) translateY(-5px); }
  70% { transform: rotate(7deg) translateX(18px) translateY(8px); }
  80% { transform: rotate(-5deg) translateX(-12px) translateY(-12px); }
  90% { transform: rotate(3deg) translateX(8px) translateY(5px); }
  100% { transform: rotate(0deg) translateX(0); }
}
@keyframes anim-heartbeat {
  0%, 100% { transform: scale(1); }
  15% { transform: scale(1.28); }
  30% { transform: scale(1); }
  45% { transform: scale(1.2); }
  60% { transform: scale(1); }
}
@keyframes anim-rocket {
  0% { transform: translateY(0) rotate(0deg); }
  20% { transform: translateY(-90px) rotate(-5deg); }
  30% { transform: translateY(-110px) rotate(3deg) scale(0.88); }
  50% { transform: translateY(15px) rotate(0deg) scale(1.06); }
  60% { transform: translateY(-35px) rotate(-2deg); }
  75% { transform: translateY(8px) rotate(1deg) scale(1.02); }
  85% { transform: translateY(-12px) rotate(0deg); }
  100% { transform: translateY(0) rotate(0deg); }
}
@keyframes anim-rock {
  0% { transform: rotate(-15deg); }
  100% { transform: rotate(15deg); }
}
@keyframes anim-wave {
  0%, 100% { transform: translateY(0) rotate(0deg); }
  25% { transform: translateY(-25px) rotate(3deg); }
  50% { transform: translateY(0) rotate(0deg); }
  75% { transform: translateY(20px) rotate(-3deg); }
}
@keyframes anim-flip {
  0%, 60%, 100% { transform: perspective(400px) rotateY(0deg); }
  70% { transform: perspective(400px) rotateY(180deg) scale(0.85); }
  85% { transform: perspective(400px) rotateY(360deg) scale(1.05); }
  95% { transform: perspective(400px) rotateY(360deg) scale(0.98); }
}
@keyframes anim-ghost {
  0%, 100% { opacity: 1; transform: translateY(0) scale(1); }
  25% { opacity: 0.2; transform: translateY(-20px) scale(0.95); }
  50% { opacity: 1; transform: translateX(15px) scale(1.05); }
  75% { opacity: 0.3; transform: translateY(15px) translateX(-10px) scale(0.9); }
}
@keyframes anim-tornado {
  0% { transform: rotate(0deg) scale(1) translateX(0); }
  25% { transform: rotate(90deg) scale(0.7) translateX(30px); }
  50% { transform: rotate(180deg) scale(1.1) translateX(0); }
  75% { transform: rotate(270deg) scale(0.8) translateX(-30px); }
  100% { transform: rotate(360deg) scale(1) translateX(0); }
}
@keyframes anim-spring {
  0%, 100% { transform: scaleY(1) scaleX(1) translateY(0); }
  20% { transform: scaleY(0.6) scaleX(1.3) translateY(30px); }
  40% { transform: scaleY(1.3) scaleX(0.8) translateY(-40px); }
  55% { transform: scaleY(0.8) scaleX(1.15) translateY(15px); }
  70% { transform: scaleY(1.1) scaleX(0.9) translateY(-15px); }
  85% { transform: scaleY(0.95) scaleX(1.05) translateY(5px); }
}
@keyframes anim-disco {
  0% { transform: rotate(0deg) scale(1); filter: hue-rotate(0deg) brightness(1); }
  12.5% { transform: rotate(45deg) scale(1.1); filter: hue-rotate(45deg) brightness(1.3); }
  25% { transform: rotate(0deg) scale(0.9); filter: hue-rotate(90deg) brightness(0.8); }
  37.5% { transform: rotate(-30deg) scale(1.15); filter: hue-rotate(135deg) brightness(1.4); }
  50% { transform: rotate(15deg) scale(1); filter: hue-rotate(180deg) brightness(1); }
  62.5% { transform: rotate(-45deg) scale(1.1); filter: hue-rotate(225deg) brightness(1.3); }
  75% { transform: rotate(20deg) scale(0.95); filter: hue-rotate(270deg) brightness(0.9); }
  87.5% { transform: rotate(-15deg) scale(1.05); filter: hue-rotate(315deg) brightness(1.2); }
  100% { transform: rotate(0deg) scale(1); filter: hue-rotate(360deg) brightness(1); }
}
@keyframes anim-worm {
  0%, 100% { transform: scaleX(1) scaleY(1) translateX(0); }
  20% { transform: scaleX(1.3) scaleY(0.75) translateX(20px); }
  40% { transform: scaleX(0.8) scaleY(1.2) translateX(35px); }
  60% { transform: scaleX(1.25) scaleY(0.8) translateX(15px); }
  80% { transform: scaleX(0.9) scaleY(1.1) translateX(5px); }
}
@keyframes anim-pendulum {
  0%, 100% { transform: rotate(0deg); }
  25% { transform: rotate(30deg); }
  75% { transform: rotate(-30deg); }
}
@keyframes anim-pixel {
  0% { transform: translate(0, 0); }
  25% { transform: translate(-3px, 2px); }
  50% { transform: translate(2px, -3px); }
  75% { transform: translate(-2px, -1px); }
  100% { transform: translate(3px, 1px); }
}
`;

export default function HomePage() {
  const AnimComponent = useMemo(() => {
    return ANIMATIONS[Math.floor(Math.random() * ANIMATIONS.length)];
  }, []);

  // 首页容器需要填满 main-content 可用空间
  // header 48px + padding 上下各 24px (var(--space-6))
  const containerStyle: React.CSSProperties = {
    position: 'relative',
    width: '100%',
    height: 'calc(100vh - 48px - 48px)',
    overflow: 'hidden',
    display: 'flex',
    justifyContent: 'center',
    alignItems: 'center',
  };

  return (
    <div className="page" style={{ margin: 'calc(-1 * var(--space-6))', padding: 0 }}>
      <style>{KEYFRAMES}</style>
      <div style={containerStyle}>
        <AnimComponent />
      </div>
    </div>
  );
}
