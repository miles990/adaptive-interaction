// MixerRenderer：把「renderer.setAnimation(name)」轉成 machine event 的 RendererBackend 門面。
//
// 用途：舊 sprite 相容 adapter（character/adapters/sprite.ts）自己呼叫
// renderer.setAnimation 來演 intent；host 把這個門面當 renderer 注入，動畫請求就
// 進入同一台 machine（與本機互動、Director 表演一起走優先階梯），真正的畫面
// 由 host 的 syncPose 依 pose() 驅動實際 renderer。這樣不會出現兩個來源互搶
// setAnimation（intent 演到一半被 500ms pump 的 pose 蓋回 idle）。
//
// 兩條不變量在這一層強制（呈現層沒有權限主權）：
//   1. adapter 送進來的 `clear-transient` 一律**去掉 force**：force 是 estop
//      clear-all 的權力，只有可信 host 路徑（CompanionApp 直接 apply）能用。
//      adapter 不得靠 setAnimation("idle") 抹掉 blocked／failed／unknown／
//      requesting-consent（對抗審查 renderer-lifecycle-028）。
//   2. pause()／resume() 必須真的轉給底下的 renderer，否則 CPP §7「看不見就不畫、
//      不排 rAF」在正式接線下是空話（對抗審查 renderer-lifecycle-029）。

import { MachineEvent, MachineState, machineEventForAnimation, MixerPort } from "./machine";
import type { MicroMotionOverlay, RendererBackend } from "./renderer";

/** adapter 可達的事件一律降權：clear-transient 不得帶 force。 */
function withoutAdapterForce(event: MachineEvent): MachineEvent {
  if (event.type !== "clear-transient") return event;
  const { force: _force, ...rest } = event;
  void _force;
  return rest;
}

export class MixerRenderer implements RendererBackend {
  constructor(
    private readonly real: RendererBackend,
    private readonly mixer: Pick<MixerPort, "apply">
  ) {}

  /** 套用後的狀態；adapter 用它判斷自己有沒有真的上台（沒上台不得回 started）。 */
  applyAnimation(name: string, frameSlice?: [number, number]): MachineState {
    return this.mixer.apply(withoutAdapterForce(machineEventForAnimation(name, frameSlice)));
  }

  setAnimation(name: string, frameSlice?: [number, number]): void {
    this.applyAnimation(name, frameSlice);
  }

  setReducedMotion(on: boolean): void {
    this.real.setReducedMotion(on);
  }

  setMicroMotion(motion: MicroMotionOverlay): void {
    this.real.setMicroMotion(motion);
  }

  pause(): void {
    this.real.pause?.();
  }

  resume(): void {
    this.real.resume?.();
  }

  destroy(): void {
    this.real.destroy();
  }
}
