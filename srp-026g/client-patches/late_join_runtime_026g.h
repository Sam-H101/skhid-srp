#ifndef LATE_JOIN_RUNTIME_026G_H
#define LATE_JOIN_RUNTIME_026G_H

typedef unsigned char lj_u8;
typedef unsigned int lj_u32;

struct lj_state;
struct lj_context;

/* Native adapters must execute begin/complete/activate on the game's thread.
 * Player destruction may arrive from a resource thread. No adapter is called
 * while the list lock is held. The asynchronous query owns its result refs
 * through the return from lj_complete, as native queries_result does.
 */
struct lj_api {
    void *(*allocate)(lj_u32);
    void (*free)(void *);
    int (*decode)(const lj_u8 *, lj_u32, void *, void *dictionary);
    void (*retain_world)(void *);
    void (*release_world)(void *);
    void (*query)(struct lj_context *);
    /* Install must acquire slot references before returning success. It may
     * fail only before modifying live slots. It does not activate the slot. */
    int (*install)(struct lj_context *);
    void (*ack)(void *client, lj_u32 seed, lj_u32 serial, lj_u8 place, lj_u8 status);
};

struct lj_state {
    lj_u8 lock;
    struct lj_context *contexts;
    struct lj_api api;
    void *client, *world, *dictionary;
    lj_u32 seed, roster;
};

struct lj_context {
    struct lj_context *next;
    struct lj_state *state;
    void *client, *world;
    lj_u32 seed, serial, place, squad;
    lj_u32 pending, owned, status, kit_mask;
    void *players[3];
    /* Stable storage for the entire lifetime of every loaded player. */
    lj_u32 profile[0x45fc / 4];
    lj_u32 wire_size;
    lj_u8 wire[];
};

#if __SIZEOF_POINTER__ == 4
/* The native NASM query adapter uses these offsets. */
_Static_assert(__builtin_offsetof(struct lj_context, kit_mask) == 44, "kit mask ABI");
_Static_assert(__builtin_offsetof(struct lj_context, place) == 24, "place ABI");
_Static_assert(__builtin_offsetof(struct lj_context, profile) == 60, "profile ABI");
#endif

enum { LJ_INSTALLED = 0, LJ_FAILED = 1, LJ_BUSY = 2 };

/* Activate once a map is ready. Deactivate before native client destruction
 * or replacing a match. Old query callbacks may still complete afterwards. */
void lj_activate(struct lj_state *, void *client, void *world, void *dictionary,
                 lj_u32 seed, lj_u32 roster);
void lj_deactivate(struct lj_state *);
/* Entire E4 extension payload (including its 14-byte header). */
void lj_prepare(struct lj_state *, const lj_u8 *, lj_u32);
/* Player pointers indexed by kit, NULL for unused/failed kits. */
void lj_complete(struct lj_context *, void *const players[3]);
/* Called after the native player destructor, with the saved profile pointer.
 * Resource memory may already be invalid: neither argument is dereferenced. */
void lj_player_destroyed(struct lj_state *, void *player, const void *profile);

#endif
