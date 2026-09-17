/**
 * 插入排序（原地思路，但这里返回新数组，不改原数组）
 * @param {number[]} arr 待排序数组
 * @returns {number[]} 升序排序后的新数组
 * 复杂度：最好 O(n)（已有序），最坏/平均 O(n²)；空间 O(1) 额外（不计返回副本）
 */
function insertionSort(arr) {
  // 拷贝一份，保持纯函数语义：排序不修改调用方传入的数组
  const a = arr.slice();
  // 外层循环：从第 2 个元素开始，依次把每个元素插入左侧已排序区间
  for (let i = 1; i < a.length; i++) {
    const key = a[i]; // 当前待插入的元素
    let j = i - 1;    // 已排序区间的最后一个下标
    // 内层循环：把比 key 大的元素依次右移一位，为 key 腾出位置
    while (j >= 0 && a[j] > key) {
      a[j + 1] = a[j];
      j--;
    }
    // 循环结束时 j+1 就是 key 应该插入的位置
    a[j + 1] = key;
  }
  return a;
}

// ── 演示 ──────────────────────────────────────────────
const input = [5, 2, 9, 1, 3, 7];
console.log('before:', input.join(' '));                 // 排序前
console.log('after :', insertionSort(input).join(' '));  // 排序后
