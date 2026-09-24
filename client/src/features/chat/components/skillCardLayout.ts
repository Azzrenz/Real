/** Card geometry: the width is mirrored in skill-picker.css (.skill-tip). */
const CARD_W = 360;
const CARD_MIN_W = 240;
const CARD_GAP = 8;

/**
 * Where the skill detail card goes. Flush-right is the default; when the viewport is
 * too narrow to fit it there, flip to the left side, and shrink only as a last resort.
 * The card must never land on top of the popover -- covering the list the pointer is
 * still moving through reads as a defect rather than as a panel.
 */
export function placeCard(
  pop: { left: number; right: number },
  vw: number,
): { left: number; width: number } {
  const rightRoom = vw - CARD_GAP - (pop.right + CARD_GAP);
  const leftRoom = pop.left - CARD_GAP - CARD_GAP;
  if (rightRoom >= CARD_W) return { left: pop.right + CARD_GAP, width: CARD_W };
  if (leftRoom >= CARD_W) return { left: pop.left - CARD_GAP - CARD_W, width: CARD_W };
  if (rightRoom >= leftRoom) {
    return { left: pop.right + CARD_GAP, width: Math.max(CARD_MIN_W, rightRoom) };
  }
  const width = Math.max(CARD_MIN_W, leftRoom);
  return { left: Math.max(CARD_GAP, pop.left - CARD_GAP - width), width };
}
