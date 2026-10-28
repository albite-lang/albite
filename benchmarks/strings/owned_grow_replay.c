/* The owned concat's grow, replayed against the C allocator (ADR 16.10.26j D4).
 *
 * Run by `make devcontainer-native-concat-cost MODE=replay`, statically linked
 * against the container's glibc. One process per (policy, shape), because the
 * heap's history is what decides where a block moves.
 *
 * Per round: one fresh join of a seed and a piece (the core's block), then
 * seven owned grows of one piece each, never freed; 200,000 rounds — the
 * sequence of 13.10.26p's matrix.c and of string_concat_owned.tg.
 *
 *   usage: owned_grow_replay <policy> <piece> <seed a> <seed b> [pad]
 *   policy: rust        exact `len` requests, `realloc` on every grow
 *                       (Rust's tg_string_concat_owned: the reference)
 *           today       the core's block, round_up_8(len + 1) + 16, `realloc`
 *                       on every grow (__albite_realloc)
 *           usable      the core's block; no call when the usable size holds it
 *           geo<n>/<d>  the core's block; no call when the usable size holds
 *                       it, else realloc(max(new, n/d * usable))
 *                       (__albite_realloc_amortised at F = n/d)
 *           cap<n>/<d>  as geo, capped: max(new, min(n/d * usable,
 *                       usable + 4096)) — D4's capped-slack variant
 *
 * F is the integer ratio n/d with a saturating multiply, rounding as
 * `amortised_request` in albite_runtime/src/alloc/amortised.rs does.
 *
 * `pad` bytes are allocated before the first round, so the caller can sweep
 * the heap's starting offset (MODE=replay sweeps it; see below).
 *
 * Output, one line: policy piece seeds pad calls moves moves/round
 * bytes_kept ns/round, where bytes kept is the final blocks' summed usable
 * size.
 */
#include <malloc.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#define ROUNDS 200000L
#define GROWS 7
#define CAP_SLACK 4096

static size_t core_block(size_t len) { return ((len + 1 + 7) & ~(size_t)7) + 16; }

/* have * n / d, saturating at SIZE_MAX — the Rust side's arithmetic. */
static size_t scaled(size_t have, size_t n, size_t d) {
  if (n != 0 && have > SIZE_MAX / n) return SIZE_MAX;
  return have * n / d;
}

int main(int argc, char **argv) {
  if (argc < 5) {
    fprintf(stderr, "usage: %s <policy> <piece> <seed a> <seed b> [pad]\n", argv[0]);
    return 2;
  }
  const char *policy = argv[1];
  int rust = !strcmp(policy, "rust"), today = !strcmp(policy, "today");
  int usable = !strcmp(policy, "usable"), capped = 0;
  size_t num = 0, den = 1;
  if (!strncmp(policy, "geo", 3) || !strncmp(policy, "cap", 3)) {
    capped = policy[0] == 'c';
    if (sscanf(policy + 3, "%zu/%zu", &num, &den) != 2 || den == 0) return 2;
  }
  if (!rust && !today && !usable && num == 0) return 2;
  size_t piece = (size_t)atol(argv[2]);
  size_t seeds[2] = {(size_t)atol(argv[3]), (size_t)atol(argv[4])};
  /* The heap's starting offset decides where blocks move (P0.2 found the
     same binary's counts change with its directory's length, which static
     glibc allocates at startup), so the start is an input: `pad` bytes
     allocated, and kept, before the first round. */
  long pad = argc > 5 ? atol(argv[5]) : 0;
  if (pad > 0 && malloc((size_t)pad) == NULL) return 3;

  long moves = 0, calls = 0;
  size_t kept = 0;
  struct timespec start, end;
  clock_gettime(CLOCK_MONOTONIC, &start);
  for (long round = 0; round < ROUNDS; round++) {
    size_t len = seeds[round % 2] + piece;
    char *block = malloc(rust ? len : core_block(len));
    for (int grow = 0; grow < GROWS; grow++) {
      len += piece;
      size_t need = rust ? len : core_block(len), ask = need;
      if (usable || num) {
        size_t have = malloc_usable_size(block);
        if (need <= have) continue;
        if (num) {
          size_t room = scaled(have, num, den);
          if (capped && room > have + CAP_SLACK) room = have + CAP_SLACK;
          if (room > ask) ask = room;
        }
      }
      char *grown = realloc(block, ask);
      if (grown == NULL) return 3;
      calls++;
      if (grown != block) moves++;
      block = grown;
    }
    block[0] = 1;
    kept += malloc_usable_size(block);
  }
  clock_gettime(CLOCK_MONOTONIC, &end);
  double ns = (double)(end.tv_sec - start.tv_sec) * 1e9 + (double)(end.tv_nsec - start.tv_nsec);
  printf("%s %zu %zu/%zu %ld %ld %ld %.2f %zu %.1f\n", policy, piece, seeds[0], seeds[1], pad, calls, moves,
         (double)moves / ROUNDS, kept, ns / ROUNDS);
  return 0;
}
