import { create } from "zustand";

export type ToastKind = "info" | "warning" | "error" | "success";

interface ToastState {
  message: string | null;
  kind: ToastKind;
  show: (message: string, kind?: ToastKind) => void;
  dismiss: () => void;
}

export const useToastStore = create<ToastState>((set) => ({
  message: null,
  kind: "info",
  show: (message, kind = "info") => set({ message, kind }),
  dismiss: () => set({ message: null }),
}));
