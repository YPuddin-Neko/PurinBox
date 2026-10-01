import { useEffect, useRef, useMemo } from 'react';

// ===== Logo 组件 =====
const Logo = ({ style, imgStyle }: { style?: React.CSSProperties; imgStyle?: React.CSSProperties }) => (
  <div style={{ display: 'inline-block', ...style }}>
    <img src="/logo.png" alt="PurinBox"
      style={{ maxWidth: 280, width: '100%', height: 'auto', objectFit: 'contain', pointerEvents: 'none', ...imgStyle }}
      draggable={false} />
  </div>
);

/** DVD 弹弹乐：位置逐帧计算，直接写到 DOM 样式上，不触发 React 重渲染 */
function DvdBounce() {
  const containerRef = useRef<HTMLDivElement>(null);
  const logoRef = useRef<HTMLDivElement>(null);
  const pos = useRef({ x: 60, y: 40 });
  const vel = useRef({ vx: 2.2, vy: 1.6 });
  const hue = useRef(0);
  const squash = useRef({ sx: 1, sy: 1, decay: 0 });
  const initialized = useRef(false);

  useEffect(() => {
    let raf: number;
    const tick = () => {
      const c = containerRef.current, l = logoRef.current;
      if (!c || !l) { raf = requestAnimationFrame(tick); return; }
      const cw = c.clientWidth, ch = c.clientHeight, lw = l.clientWidth, lh = l.clientHeight;
      const p = pos.current, v = vel.current, sq = squash.current;
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
      const st = l.style;
      st.left = `${p.x}px`;
      st.top = `${p.y}px`;
      st.transform = `scale(${sq.sx}, ${sq.sy})`;
      st.filter = `hue-rotate(${hue.current}deg)`;
      st.transition = 'filter 0.4s';
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, []);

  return (
    <div ref={containerRef} style={{ position: 'absolute', inset: 0, overflow: 'hidden' }}>
      <div ref={logoRef} style={{ display: 'inline-block', position: 'absolute', left: '50%', top: '50%', transform: 'translate(-50%, -50%)' }}>
        <Logo />
      </div>
    </div>
  );
}

// ===== 50 种搞怪动画 =====
// 'dvd' 走上面的 DvdBounce，其余都是给 Logo 套一段 CSS 动画
type LogoAnim = { style: React.CSSProperties; imgStyle?: React.CSSProperties };

const ANIMATIONS: (LogoAnim | 'dvd')[] = [
  { style: { animation: 'anim-spin 1.2s linear infinite' } }, // 鸡块旋转
  'dvd', // DVD 弹弹乐
  { style: { animation: 'anim-jelly 1.5s ease-in-out infinite' } }, // 果冻抖动
  { style: { animation: 'anim-drunk 2.5s ease-in-out infinite' } }, // 醉酒摇摆
  { style: { animation: 'anim-heartbeat 1s ease-in-out infinite' } }, // 心跳膨胀
  { style: { animation: 'anim-rocket 1.8s cubic-bezier(0.34, 1.56, 0.64, 1) infinite' } }, // 火箭蹦跳
  { style: { animation: 'anim-rock 0.6s ease-in-out infinite alternate', transformOrigin: 'center bottom' } }, // 摇滚摇摆
  { style: { animation: 'anim-wave 3s ease-in-out infinite' } }, // 海浪漂浮
  { style: { animation: 'anim-flip 2s ease-in-out infinite' } }, // 翻跟斗
  { style: { animation: 'anim-ghost 2.5s ease-in-out infinite' } }, // 幽灵闪现
  { style: { animation: 'anim-tornado 2s linear infinite' } }, // 龙卷风
  { style: { animation: 'anim-spring 1s ease-in-out infinite' } }, // 弹簧弹跳
  { style: { animation: 'anim-disco 0.8s steps(8) infinite' }, imgStyle: { filter: 'saturate(2)' } }, // 迪斯科
  { style: { animation: 'anim-worm 1.5s ease-in-out infinite' } }, // 毛毛虫蠕动
  { style: { animation: 'anim-pendulum 2s ease-in-out infinite', transformOrigin: 'center top' } }, // 钟摆摇
  { style: { animation: 'anim-pixel 0.1s steps(2) infinite' }, imgStyle: { imageRendering: 'pixelated' } }, // 像素抖动
  { style: { animation: 'anim-breathe 3s ease-in-out infinite' } }, // 呼吸灯 — 缓慢明暗变化
  { style: { animation: 'anim-rubber 1.2s ease-in-out infinite' } }, // 橡皮筋 — 拉伸弹回
  { style: { animation: 'anim-glitch 0.3s steps(3) infinite' } }, // 故障闪烁 — 赛博朋克感
  { style: { animation: 'anim-space 6s ease-in-out infinite' } }, // 太空漂移 — 失重旋转飘
  { style: { animation: 'anim-jump 0.8s cubic-bezier(0.33, 1, 0.68, 1) infinite', transformOrigin: 'center bottom' } }, // 跳绳 — 原地跳跃
  { style: { animation: 'anim-typewriter 2s steps(20) infinite alternate', overflow: 'hidden' } }, // 打字机 — 从左展开
  { style: { animation: 'anim-inflate 2s ease-in-out infinite' } }, // 膨胀爆炸 — 膨胀后缩回
  { style: { animation: 'anim-flipx 2.5s ease-in-out infinite' } }, // 3D翻转 — X轴翻转
  { style: { animation: 'anim-wiggle 0.5s ease-in-out infinite' } }, // 抖一抖 — 快速水平抖动
  { style: { animation: 'anim-pop 1.5s cubic-bezier(0.68, -0.55, 0.27, 1.55) infinite' } }, // 弹出 — pop效果
  { style: { animation: 'anim-tilt3d 3s ease-in-out infinite' } }, // 3D倾斜 — 透视倾斜
  { style: { animation: 'anim-meteor 2.5s ease-in-out infinite' } }, // 流星 — 斜向飞过
  { style: { animation: 'anim-domino 2s ease-in-out infinite', transformOrigin: 'bottom center' } }, // 多米诺 — 倒下又立起
  { style: { animation: 'anim-accordion 1.5s ease-in-out infinite' } }, // 手风琴 — 水平压缩
  { style: { animation: 'anim-portal 2.5s ease-in-out infinite' } }, // 传送门 — 缩小消失再出现
  { style: { animation: 'anim-orbit 3s linear infinite' } }, // 轨道运动 — 椭圆轨道
  { style: { animation: 'anim-bounce-beats 0.6s ease-in-out infinite' } }, // 蹦迪 — 上下有节奏跳
  { style: { animation: 'anim-neon 1.5s ease-in-out infinite' }, imgStyle: { filter: 'drop-shadow(0 0 8px rgba(124, 92, 252, 0.8))' } }, // 霓虹闪烁 — 发光效果
  { style: { animation: 'anim-plane 3s ease-in-out infinite' } }, // 纸飞机 — 俯冲滑翔
  { style: { animation: 'anim-jellydrop 2s cubic-bezier(0.34, 1.56, 0.64, 1) infinite' } }, // 果冻落地 — 从上掉下弹几下
  { style: { animation: 'anim-carousel 3s linear infinite' } }, // 旋转木马 — Y轴持续旋转
  { style: { animation: 'anim-dice 0.6s ease-in-out infinite' } }, // 摇骰子 — 随机方向快速晃
  { style: { animation: 'anim-butterfly 3s ease-in-out infinite' } }, // 蝴蝶 — 忽上忽下飘忽
  { style: { animation: 'anim-bball 2s ease-in infinite', transformOrigin: 'center bottom' } }, // 弹力球 — 地面弹跳渐弱
  { style: { animation: 'anim-magnet 2s ease-in-out infinite' } }, // 磁铁吸引 — 左右吸引
  { style: { animation: 'anim-waterdrop 2s ease-in-out infinite', transformOrigin: 'center top' } }, // 水滴 — 拉长滴落
  { style: { animation: 'anim-tvstatic 0.15s steps(4) infinite' } }, // 电视噪点 — 快速小幅偏移+亮度变化
  { style: { animation: 'anim-seesaw 1.5s ease-in-out infinite', transformOrigin: 'center bottom' } }, // 跷跷板 — 左右交替倾斜
  { style: { animation: 'anim-spiral 3s linear infinite' } }, // 螺旋上升 — 旋转+上移
  { style: { animation: 'anim-melt 3s ease-in-out infinite', transformOrigin: 'center bottom' } }, // 熔化 — 上半透明下半扭曲
  { style: { animation: 'anim-boomerang 2s ease-in-out infinite' } }, // 回旋镖 — 飞出去再飞回来
  { style: { animation: 'anim-pulse-wave 2s ease-out infinite' } }, // 脉冲波 — 放大+透明度扩散
  { style: { animation: 'anim-flipbook 2s ease-in-out infinite' } }, // 翻书 — Z轴旋转翻页感
  { style: { animation: 'anim-eight 4s ease-in-out infinite' } }, // 8字形 — 走∞路线
];

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
@keyframes anim-breathe {
  0%, 100% { transform: scale(1); opacity: 1; }
  50% { transform: scale(1.08); opacity: 0.7; }
}
@keyframes anim-rubber {
  0% { transform: scale(1, 1); }
  15% { transform: scale(1.35, 0.75); }
  25% { transform: scale(0.75, 1.25); }
  35% { transform: scale(1.2, 0.85); }
  45% { transform: scale(0.9, 1.1); }
  55% { transform: scale(1.05, 0.95); }
  65% { transform: scale(0.98, 1.02); }
  100% { transform: scale(1, 1); }
}
@keyframes anim-glitch {
  0% { transform: translate(0); filter: hue-rotate(0deg); }
  20% { transform: translate(-5px, 3px) skewX(5deg); filter: hue-rotate(90deg); }
  40% { transform: translate(4px, -2px) skewX(-3deg); filter: hue-rotate(180deg); }
  60% { transform: translate(-3px, -4px) skewX(2deg); filter: hue-rotate(270deg); }
  80% { transform: translate(5px, 2px) skewX(-5deg); filter: hue-rotate(45deg); }
  100% { transform: translate(0); filter: hue-rotate(0deg); }
}
@keyframes anim-space {
  0% { transform: translate(0, 0) rotate(0deg) scale(1); }
  15% { transform: translate(30px, -20px) rotate(15deg) scale(0.9); }
  30% { transform: translate(-20px, -40px) rotate(-10deg) scale(1.1); }
  45% { transform: translate(40px, 10px) rotate(25deg) scale(0.85); }
  60% { transform: translate(-30px, 30px) rotate(-20deg) scale(1.05); }
  75% { transform: translate(15px, -15px) rotate(10deg) scale(0.95); }
  90% { transform: translate(-10px, 20px) rotate(-5deg) scale(1.02); }
  100% { transform: translate(0, 0) rotate(0deg) scale(1); }
}
@keyframes anim-jump {
  0%, 100% { transform: translateY(0) scaleY(1) scaleX(1); }
  10% { transform: translateY(0) scaleY(0.8) scaleX(1.15); }
  30% { transform: translateY(-70px) scaleY(1.15) scaleX(0.9); }
  50% { transform: translateY(-80px) scaleY(1.05) scaleX(0.95); }
  70% { transform: translateY(0) scaleY(0.85) scaleX(1.1); }
  80% { transform: translateY(-8px) scaleY(1.02) scaleX(0.98); }
}
@keyframes anim-typewriter {
  0% { clip-path: inset(0 100% 0 0); }
  100% { clip-path: inset(0 0 0 0); }
}
@keyframes anim-inflate {
  0%, 100% { transform: scale(1); }
  30% { transform: scale(1.4); }
  50% { transform: scale(1.45); filter: brightness(1.1); }
  70% { transform: scale(1.4); }
}
@keyframes anim-flipx {
  0%, 100% { transform: perspective(800px) rotateX(0deg); }
  50% { transform: perspective(800px) rotateX(360deg); }
}
@keyframes anim-wiggle {
  0%, 100% { transform: rotate(0deg); }
  10% { transform: rotate(6deg); }
  20% { transform: rotate(-6deg); }
  30% { transform: rotate(5deg); }
  40% { transform: rotate(-5deg); }
  50% { transform: rotate(4deg); }
  60% { transform: rotate(-4deg); }
  70% { transform: rotate(2deg); }
  80% { transform: rotate(-2deg); }
  90% { transform: rotate(1deg); }
}
@keyframes anim-pop {
  0%, 100% { transform: scale(1); }
  15% { transform: scale(0); }
  30% { transform: scale(1.3); }
  45% { transform: scale(0.9); }
  60% { transform: scale(1.1); }
  75% { transform: scale(0.97); }
}
@keyframes anim-tilt3d {
  0%, 100% { transform: perspective(600px) rotateY(0deg) rotateX(0deg); }
  25% { transform: perspective(600px) rotateY(25deg) rotateX(10deg); }
  50% { transform: perspective(600px) rotateY(0deg) rotateX(-10deg); }
  75% { transform: perspective(600px) rotateY(-25deg) rotateX(5deg); }
}
@keyframes anim-meteor {
  0% { transform: translate(-200px, -100px) rotate(-30deg) scale(0.5); opacity: 0; }
  20% { opacity: 1; }
  50% { transform: translate(0, 0) rotate(0deg) scale(1); opacity: 1; }
  80% { opacity: 1; }
  100% { transform: translate(200px, 100px) rotate(30deg) scale(0.5); opacity: 0; }
}
@keyframes anim-domino {
  0%, 100% { transform: rotateX(0deg); }
  30% { transform: rotateX(80deg); }
  40% { transform: rotateX(70deg); }
  60% { transform: rotateX(80deg); }
  80% { transform: rotateX(0deg); }
}
@keyframes anim-accordion {
  0%, 100% { transform: scaleX(1); }
  25% { transform: scaleX(0.4); }
  50% { transform: scaleX(1.2); }
  75% { transform: scaleX(0.6); }
}
@keyframes anim-portal {
  0%, 100% { transform: scale(1) rotate(0deg); opacity: 1; }
  25% { transform: scale(0.1) rotate(180deg); opacity: 0.3; }
  50% { transform: scale(0.1) rotate(360deg); opacity: 0; }
  75% { transform: scale(0.5) rotate(540deg); opacity: 0.5; }
}
@keyframes anim-orbit {
  0% { transform: translateX(60px) translateY(0) rotate(0deg); }
  25% { transform: translateX(0) translateY(-40px) rotate(90deg); }
  50% { transform: translateX(-60px) translateY(0) rotate(180deg); }
  75% { transform: translateX(0) translateY(40px) rotate(270deg); }
  100% { transform: translateX(60px) translateY(0) rotate(360deg); }
}
@keyframes anim-bounce-beats {
  0%, 100% { transform: translateY(0) scaleY(1); }
  30% { transform: translateY(-30px) scaleY(1.1); }
  50% { transform: translateY(0) scaleY(0.85) scaleX(1.1); }
  65% { transform: translateY(-12px) scaleY(1.05); }
  80% { transform: translateY(0) scaleY(0.95) scaleX(1.05); }
}
@keyframes anim-neon {
  0%, 100% { filter: drop-shadow(0 0 5px rgba(124,92,252,0.6)) drop-shadow(0 0 20px rgba(124,92,252,0.3)); }
  25% { filter: drop-shadow(0 0 20px rgba(124,92,252,1)) drop-shadow(0 0 60px rgba(56,189,248,0.6)); }
  50% { filter: drop-shadow(0 0 5px rgba(56,189,248,0.4)) drop-shadow(0 0 10px rgba(56,189,248,0.2)); }
  75% { filter: drop-shadow(0 0 25px rgba(245,158,11,0.8)) drop-shadow(0 0 50px rgba(124,92,252,0.5)); }
}
@keyframes anim-plane {
  0% { transform: translate(-120px, 60px) rotate(15deg) scale(0.6); }
  30% { transform: translate(20px, -30px) rotate(-5deg) scale(1.1); }
  50% { transform: translate(40px, -20px) rotate(3deg) scale(1); }
  70% { transform: translate(-10px, 10px) rotate(-8deg) scale(0.95); }
  100% { transform: translate(-120px, 60px) rotate(15deg) scale(0.6); }
}
@keyframes anim-jellydrop {
  0% { transform: translateY(-150px) scaleY(1.3) scaleX(0.8); }
  25% { transform: translateY(10px) scaleY(0.7) scaleX(1.3); }
  40% { transform: translateY(-30px) scaleY(1.15) scaleX(0.9); }
  55% { transform: translateY(5px) scaleY(0.85) scaleX(1.1); }
  70% { transform: translateY(-10px) scaleY(1.05) scaleX(0.95); }
  85% { transform: translateY(2px) scaleY(0.98) scaleX(1.02); }
  100% { transform: translateY(0) scaleY(1) scaleX(1); }
}
@keyframes anim-carousel {
  0% { transform: perspective(600px) rotateY(0deg); }
  100% { transform: perspective(600px) rotateY(360deg); }
}
@keyframes anim-dice {
  0%, 100% { transform: rotate(0deg) translate(0, 0); }
  10% { transform: rotate(12deg) translate(8px, -5px); }
  20% { transform: rotate(-10deg) translate(-6px, 4px); }
  30% { transform: rotate(8deg) translate(5px, -8px); }
  40% { transform: rotate(-12deg) translate(-8px, 3px); }
  50% { transform: rotate(15deg) translate(6px, -4px); }
  60% { transform: rotate(-8deg) translate(-5px, 7px); }
  70% { transform: rotate(6deg) translate(4px, -3px); }
  80% { transform: rotate(-4deg) translate(-3px, 2px); }
  90% { transform: rotate(2deg) translate(1px, -1px); }
}
@keyframes anim-butterfly {
  0%, 100% { transform: translate(0, 0) rotate(0deg); }
  15% { transform: translate(25px, -35px) rotate(5deg); }
  30% { transform: translate(-15px, -20px) rotate(-8deg); }
  45% { transform: translate(30px, 10px) rotate(3deg); }
  60% { transform: translate(-25px, -30px) rotate(-5deg); }
  75% { transform: translate(10px, 15px) rotate(7deg); }
  90% { transform: translate(-5px, -10px) rotate(-2deg); }
}
@keyframes anim-bball {
  0% { transform: translateY(0); }
  15% { transform: translateY(-80px); }
  30% { transform: translateY(0) scaleY(0.8) scaleX(1.15); }
  35% { transform: translateY(-5px) scaleY(1) scaleX(1); }
  45% { transform: translateY(-45px); }
  60% { transform: translateY(0) scaleY(0.85) scaleX(1.1); }
  65% { transform: translateY(-3px) scaleY(1) scaleX(1); }
  75% { transform: translateY(-20px); }
  90% { transform: translateY(0) scaleY(0.9) scaleX(1.05); }
  100% { transform: translateY(0); }
}
@keyframes anim-magnet {
  0%, 100% { transform: translateX(0); }
  15% { transform: translateX(50px); }
  20% { transform: translateX(45px); }
  35% { transform: translateX(-50px); }
  40% { transform: translateX(-45px); }
  55% { transform: translateX(30px); }
  60% { transform: translateX(28px); }
  75% { transform: translateX(-30px); }
  80% { transform: translateX(-28px); }
}
@keyframes anim-waterdrop {
  0%, 100% { transform: scaleY(1) scaleX(1) translateY(0); }
  20% { transform: scaleY(1.3) scaleX(0.85) translateY(0); }
  40% { transform: scaleY(1.5) scaleX(0.75) translateY(15px); }
  55% { transform: scaleY(0.7) scaleX(1.2) translateY(30px); }
  70% { transform: scaleY(1.1) scaleX(0.95) translateY(10px); }
  85% { transform: scaleY(0.95) scaleX(1.03) translateY(3px); }
}
@keyframes anim-tvstatic {
  0% { transform: translate(0, 0); filter: brightness(1); }
  25% { transform: translate(-2px, 1px); filter: brightness(1.3); }
  50% { transform: translate(1px, -2px); filter: brightness(0.8); }
  75% { transform: translate(2px, 1px); filter: brightness(1.2); }
  100% { transform: translate(-1px, -1px); filter: brightness(0.9); }
}
@keyframes anim-seesaw {
  0%, 100% { transform: rotate(0deg); }
  25% { transform: rotate(20deg); }
  50% { transform: rotate(0deg); }
  75% { transform: rotate(-20deg); }
}
@keyframes anim-spiral {
  0% { transform: translateY(40px) rotate(0deg) scale(0.8); opacity: 0.5; }
  50% { transform: translateY(-40px) rotate(360deg) scale(1.1); opacity: 1; }
  100% { transform: translateY(40px) rotate(720deg) scale(0.8); opacity: 0.5; }
}
@keyframes anim-melt {
  0%, 100% { transform: scaleY(1) skewX(0deg); filter: blur(0); }
  30% { transform: scaleY(0.85) skewX(3deg); filter: blur(0.5px); }
  50% { transform: scaleY(0.7) skewX(-5deg); filter: blur(1px); }
  70% { transform: scaleY(0.85) skewX(2deg); filter: blur(0.5px); }
}
@keyframes anim-boomerang {
  0%, 100% { transform: translate(0, 0) rotate(0deg); }
  25% { transform: translate(100px, -30px) rotate(180deg) scale(0.7); }
  50% { transform: translate(150px, 0) rotate(360deg) scale(0.5); }
  75% { transform: translate(100px, 30px) rotate(540deg) scale(0.7); }
}
@keyframes anim-pulse-wave {
  0% { transform: scale(1); opacity: 1; filter: drop-shadow(0 0 0 rgba(124,92,252,0)); }
  50% { transform: scale(1.15); opacity: 0.8; filter: drop-shadow(0 0 20px rgba(124,92,252,0.5)); }
  100% { transform: scale(1); opacity: 1; filter: drop-shadow(0 0 0 rgba(124,92,252,0)); }
}
@keyframes anim-flipbook {
  0%, 100% { transform: perspective(500px) rotateY(0deg) rotateZ(0deg); }
  20% { transform: perspective(500px) rotateY(60deg) rotateZ(5deg); }
  40% { transform: perspective(500px) rotateY(120deg) rotateZ(-3deg); }
  60% { transform: perspective(500px) rotateY(240deg) rotateZ(3deg); }
  80% { transform: perspective(500px) rotateY(300deg) rotateZ(-5deg); }
}
@keyframes anim-eight {
  0% { transform: translate(0, 0); }
  12.5% { transform: translate(50px, -25px); }
  25% { transform: translate(80px, 0); }
  37.5% { transform: translate(50px, 25px); }
  50% { transform: translate(0, 0); }
  62.5% { transform: translate(-50px, 25px); }
  75% { transform: translate(-80px, 0); }
  87.5% { transform: translate(-50px, -25px); }
  100% { transform: translate(0, 0); }
}
`;

export default function HomePage() {
  const anim = useMemo(() => ANIMATIONS[Math.floor(Math.random() * ANIMATIONS.length)], []);

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
        {anim === 'dvd' ? <DvdBounce /> : <Logo style={anim.style} imgStyle={anim.imgStyle} />}
      </div>
    </div>
  );
}
