/*
 * hw/zenoh/zenoh-chardev.c — Rust-backed Deterministic Serial Backend
 *
 * SPDX-License-Identifier: GPL-2.0-or-later
 */
#include "qemu/osdep.h"
#include "chardev/char.h"
#include "qapi/error.h"
#include "qapi/qapi-types-char.h"
#include "qemu/timer.h"
#include "qom/object.h"
#include "qemu/module.h"
#include "qemu/option.h"

/* ── Rust FFI declarations ────────────────────────────────────────────────── */

typedef struct ZenohChardevState ZenohChardevState;

extern ZenohChardevState *zenoh_chardev_init(Chardev *chr, const char *node_id, const char *router, const char *topic);
extern void              zenoh_chardev_cleanup_rust(ZenohChardevState *state);
extern int               zenoh_chardev_write_rust(ZenohChardevState *state, const uint8_t *buf, int len);

#define TYPE_CHARDEV_ZENOH "chardev-zenoh"
OBJECT_DECLARE_SIMPLE_TYPE(ChardevZenoh, CHARDEV_ZENOH)

struct ChardevZenoh {
    Chardev parent;
    ZenohChardevState *rust_state;
};

static int zenoh_chr_write(Chardev *chr, const uint8_t *buf, int len)
{
    ChardevZenoh *s = CHARDEV_ZENOH(chr);
    return zenoh_chardev_write_rust(s->rust_state, buf, len);
}

static void zenoh_chr_parse(QemuOpts *opts, ChardevBackend *backend, Error **errp)
{
    const char *node = qemu_opt_get(opts, "node");
    const char *router = qemu_opt_get(opts, "router");
    const char *topic = qemu_opt_get(opts, "topic");

    if (!node) {
        error_setg(errp, "chardev: zenoh: 'node' is required");
        return;
    }

    ChardevZenohOptions *zenoh_opts = g_new0(ChardevZenohOptions, 1);
    zenoh_opts->node = g_strdup(node);
    if (router) {
        zenoh_opts->router = g_strdup(router);
    }
    if (topic) {
        zenoh_opts->topic = g_strdup(topic);
    }

    backend->type = CHARDEV_BACKEND_KIND_ZENOH;
    backend->u.zenoh.data = zenoh_opts;
    
    qemu_chr_parse_common(opts, qapi_ChardevZenohOptions_base(zenoh_opts));
}

static bool zenoh_chr_open(Chardev *chr, ChardevBackend *backend, Error **errp)
{
    ChardevZenoh *s = CHARDEV_ZENOH(chr);
    ChardevZenohOptions *opts = backend->u.zenoh.data;
    
    s->rust_state = zenoh_chardev_init(chr, opts->node, opts->router, opts->topic);
    if (!s->rust_state) {
        error_setg(errp, "zenoh-chardev: failed to initialize Rust backend (check Zenoh router/connectivity)");
        return false;
    }
    
    return true;
}

static void zenoh_chr_finalize(Object *obj)
{
    ChardevZenoh *s = CHARDEV_ZENOH(obj);
    if (s->rust_state) {
        zenoh_chardev_cleanup_rust(s->rust_state);
        s->rust_state = NULL;
    }
}

static void char_zenoh_class_init(ObjectClass *oc, const void *data)
{
    ChardevClass *cc = CHARDEV_CLASS(oc);

    cc->chr_parse = zenoh_chr_parse;
    cc->chr_open = zenoh_chr_open;
    cc->chr_write = zenoh_chr_write;
}

static const TypeInfo char_zenoh_type_info = {
    .name = TYPE_CHARDEV_ZENOH,
    .parent = TYPE_CHARDEV,
    .instance_size = sizeof(ChardevZenoh),
    .instance_finalize = zenoh_chr_finalize,
    .class_init = char_zenoh_class_init,
};

static const TypeInfo char_zenoh_types[] = { char_zenoh_type_info };

DEFINE_TYPES(char_zenoh_types)
module_obj(TYPE_CHARDEV_ZENOH);
