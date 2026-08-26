// One icon library (Lucide), individually imported. The same concept always
// uses the same icon; unknown hints fall back by capability kind.

import React from "react";
import {
  Activity,
  AudioLines,
  BadgeCheck,
  Bell,
  Bot,
  Calendar,
  CalendarPlus,
  Cat,
  CircleCheck,
  CircleHelp,
  CircleX,
  Clock,
  CloudDownload,
  CloudUpload,
  Code2,
  Cpu,
  Eye,
  File,
  FileText,
  FlaskConical,
  FolderOpen,
  Hand,
  History,
  House,
  Keyboard,
  Laptop,
  Lightbulb,
  ListChecks,
  LoaderCircle,
  LockKeyhole,
  MapPin,
  MessageSquare,
  Mic,
  OctagonX,
  PanelTop,
  Pause,
  Pencil,
  Play,
  Plus,
  Radio,
  ScanEye,
  Search,
  Send,
  SendHorizontal,
  Settings,
  ShieldCheck,
  ShieldX,
  Thermometer,
  Trash2,
  TriangleAlert,
  UserRound,
  Vibrate,
  Video,
  Volume2,
  Webhook,
  Wifi,
  Workflow,
  Wrench,
} from "lucide-react";

const ICONS: Record<string, React.ComponentType<{ size?: number | string; className?: string }>> = {
  activity: Activity,
  "audio-lines": AudioLines,
  "badge-check": BadgeCheck,
  bell: Bell,
  bot: Bot,
  calendar: Calendar,
  "calendar-plus": CalendarPlus,
  cat: Cat,
  "circle-check": CircleCheck,
  "circle-help": CircleHelp,
  "circle-x": CircleX,
  clock: Clock,
  "cloud-download": CloudDownload,
  "cloud-upload": CloudUpload,
  code2: Code2,
  cpu: Cpu,
  eye: Eye,
  file: File,
  "file-text": FileText,
  "flask-conical": FlaskConical,
  "folder-open": FolderOpen,
  hand: Hand,
  history: History,
  house: House,
  keyboard: Keyboard,
  laptop: Laptop,
  lightbulb: Lightbulb,
  "list-checks": ListChecks,
  "loader-circle": LoaderCircle,
  "lock-keyhole": LockKeyhole,
  "map-pin": MapPin,
  "message-square": MessageSquare,
  mic: Mic,
  "octagon-x": OctagonX,
  "panel-top": PanelTop,
  pause: Pause,
  pencil: Pencil,
  play: Play,
  plus: Plus,
  radio: Radio,
  "scan-eye": ScanEye,
  search: Search,
  send: Send,
  "send-horizontal": SendHorizontal,
  settings: Settings,
  "shield-check": ShieldCheck,
  "shield-x": ShieldX,
  thermometer: Thermometer,
  "trash-2": Trash2,
  "triangle-alert": TriangleAlert,
  "user-round": UserRound,
  vibrate: Vibrate,
  video: Video,
  "volume-2": Volume2,
  webhook: Webhook,
  wifi: Wifi,
  workflow: Workflow,
  wrench: Wrench,
};

/** Render an icon by catalog/manifest hint name; decorative by default. */
export function Icon({
  name,
  size = 20,
  className,
  label,
}: {
  name: string;
  size?: number;
  className?: string;
  label?: string;
}) {
  const Cmp = ICONS[name] ?? CircleHelp;
  return (
    <span
      className={className ? `icon ${className}` : "icon"}
      aria-hidden={label ? undefined : true}
      aria-label={label}
      role={label ? "img" : undefined}
    >
      <Cmp size={size} />
    </span>
  );
}
