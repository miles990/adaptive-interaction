// 角色庫摘要（M3 §4.1）：預設只列「使用中 1 張＋最近／常用 3 張」，其餘收在「顯示全部角色」後面。
//
// 純函式、不認得任何角色 id：排序只看三件事——
//   1. 使用中的角色永遠排第一（它就在畫面上，必須看得到）；
//   2. 呼叫端提供的「最近／常用」清單，依它自己的順序；
//   3. 其餘依目錄本來的順序遞補。
// 呼叫端沒有可靠的最近使用資料時就傳空陣列——這裡不會自己編一個順序出來冒充「最近」。

export const LIBRARY_DIGEST_LIMIT = 4;

export interface LibraryDigest<T> {
  /** 依上面的規則排序後、最多 `limit` 張。 */
  shown: T[];
  /** 沒有列出的張數（0 代表沒有「顯示全部」的必要）。 */
  hidden: number;
}

export function libraryDigest<T extends { characterId: string }>(
  cards: readonly T[],
  activeId: string,
  options: { usedIds?: readonly string[]; limit?: number } = {}
): LibraryDigest<T> {
  const limit = Math.max(1, options.limit ?? LIBRARY_DIGEST_LIMIT);
  const used = options.usedIds ?? [];
  const picked: T[] = [];
  const seen = new Set<string>();
  const take = (card: T | undefined) => {
    if (!card || seen.has(card.characterId) || picked.length >= limit) return;
    seen.add(card.characterId);
    picked.push(card);
  };

  take(cards.find((c) => c.characterId === activeId));
  for (const id of used) take(cards.find((c) => c.characterId === id));
  for (const card of cards) take(card);

  return { shown: picked, hidden: Math.max(0, cards.length - picked.length) };
}
