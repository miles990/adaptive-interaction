// MixerRenderer：把「renderer.setAnimation(name)」轉成 machine event 的 RendererBackend 門面。
//
// 用途：舊 sprite 相容 adapter（character/adapters/sprite.ts）自己呼叫
// renderer.setAnimation 來演 intent；host 把這個門面當 renderer 注入，動畫請求就
// 進入同一台 machine（與本機互動、Director 表演一起走優先階梯），真正的畫面
// 由 host 的 syncPose 依 pose() 驅動實際 renderer。這樣不會出現兩個來源互搶
// setAnimation（intent 演到一半被 500ms pump 的 pose 蓋回 idle）。

import { machineEventForAnimation, MixerPort } from "./machine";
import type { MicroMotionOverlay, RendererBackend } from "./renderer";

export class MixerRenderer implements RendererBackend {
  constructor(
    private readonly real: RendererBackend,
    private readonly mixer: Pick<MixerPort, "apply">
  ) {}

  setAnimation(name: string, frameSlice?: [number, number]): void {
    this.mixer.apply(machineEventForAnimation(name, frameSlice));
  }

  setReducedMotion(on: boolean): void {
    this.real.setReducedMotion(on);
  }

  setMicroMotion(motion: MicroMotionOverlay): void {
    this.real.setMicroMotion(motion);
  }

  destroy(): void {
    this.real.destroy();
  }
}
