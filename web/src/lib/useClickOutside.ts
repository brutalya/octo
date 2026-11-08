import { useEffect, type RefObject } from 'react';

/** While `active`, calls `onClose` on a pointerdown outside `ref` or on Escape. */
export function useClickOutside(ref: RefObject<HTMLElement | null>, active: boolean, onClose: () => void): void {
  useEffect(() => {
    if (!active) return;
    const onDown = (e: PointerEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    document.addEventListener('pointerdown', onDown);
    document.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('pointerdown', onDown);
      document.removeEventListener('keydown', onKey);
    };
  }, [ref, active, onClose]);
}
