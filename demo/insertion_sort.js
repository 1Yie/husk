function insertionSort(arr) {
  const a = arr.slice();
  // 外层循环：从第 2 个元素开始，依次把每个元素插入左侧已排序区间
  for (let i = 1; i < a.length; i++) {
    const key = a[i];
    let j = i - 1;
    while (j >= 0 && a[j] > key) {
      a[j + 1] = a[j];
      j--;
    }
    a[j + 1] = key;
  }
  return a;
}

const input = [5, 2, 9, 1, 3, 7];
console.log('before:', input.join(' '));
console.log('after :', insertionSort(input).join(' '));
