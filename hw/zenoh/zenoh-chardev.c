#include "qemu/osdep.h"
#include "chardev/char.h"
#include "qapi/error.h"
#include "qapi/qapi-types-char.h"
#include "qemu/timer.h"
#include "qemu/main-loop.h"
#include "qom/object.h"
#include "qemu/module.h"
#include "qemu/option.h"
#include <zenoh.h>

#define TYPE_CHARDEV_ZENOH "chardev-zenoh"
OBJECT_DECLARE_SIMPLE_TYPE(ChardevZenoh, CHARDEV_ZENOH)

struct ChardevZenoh {
    Chardev parent;
    z_owned_session_t session;
    z_owned_publisher_t publisher;
    z_owned_subscriber_t subscriber;
    QEMUTimer *rx_timer;
    char *node_id;
    char *router;

    struct {
        uint64_t delivery_vtime;
        uint8_t *data;
        size_t size;
    } rx_queue[1024];
    int rx_count;
    QemuMutex mutex;
};

typedef struct __attribute__((packed)) {
    uint64_t delivery_vtime_ns;
    uint32_t size;
} ZenohFrameHeader;

static void push_rx_frame(ChardevZenoh *s, uint64_t vtime, const uint8_t *data, size_t size)
{
    qemu_mutex_lock(&s->mutex);
    if (s->rx_count < 1024) {
        int i = s->rx_count - 1;
        while (i >= 0 && s->rx_queue[i].delivery_vtime < vtime) {
            s->rx_queue[i + 1] = s->rx_queue[i];
            i--;
        }
        s->rx_queue[i + 1].delivery_vtime = vtime;
        s->rx_queue[i + 1].data = g_memdup2(data, size);
        s->rx_queue[i + 1].size = size;
        s->rx_count++;
        
        timer_mod(s->rx_timer, s->rx_queue[s->rx_count - 1].delivery_vtime);
    }
    qemu_mutex_unlock(&s->mutex);
}

static void on_zenoh_msg(z_loaned_sample_t *sample, void *context)
{
    ChardevZenoh *s = context;
    const z_loaned_bytes_t *payload = z_sample_payload(sample);
    if (!payload) return;
    
    z_bytes_reader_t reader = z_bytes_get_reader(payload);
    ZenohFrameHeader hdr;
    if (z_bytes_reader_read(&reader, (uint8_t*)&hdr, sizeof(hdr)) != sizeof(hdr)) {
        return;
    }
    
    uint8_t *frame_data = g_malloc(hdr.size);
    if (z_bytes_reader_read(&reader, frame_data, hdr.size) == hdr.size) {
        push_rx_frame(s, hdr.delivery_vtime_ns, frame_data, hdr.size);
    }
    g_free(frame_data);
}

static void rx_timer_cb(void *opaque)
{
    ChardevZenoh *s = opaque;
    qemu_mutex_lock(&s->mutex);
    
    uint64_t now = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL);
    while (s->rx_count > 0) {
        int last = s->rx_count - 1;
        if (s->rx_queue[last].delivery_vtime <= now) {
            uint8_t *data = s->rx_queue[last].data;
            size_t size = s->rx_queue[last].size;
            s->rx_count--;
            
            qemu_mutex_unlock(&s->mutex);
            /* Push received bytes into QEMU's frontend (e.g. PL011) */
            qemu_chr_be_write(CHARDEV(s), data, size);
            g_free(data);
            qemu_mutex_lock(&s->mutex);
        } else {
            timer_mod(s->rx_timer, s->rx_queue[last].delivery_vtime);
            break;
        }
    }
    qemu_mutex_unlock(&s->mutex);
}

static int zenoh_chr_write(Chardev *chr, const uint8_t *buf, int len)
{
    ChardevZenoh *s = CHARDEV_ZENOH(chr);
    
    ZenohFrameHeader hdr = {
        .delivery_vtime_ns = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL),
        .size = len
    };
    
    uint8_t *msg = g_malloc(sizeof(hdr) + len);
    memcpy(msg, &hdr, sizeof(hdr));
    memcpy(msg + sizeof(hdr), buf, len);
    
    z_owned_bytes_t payload;
    z_bytes_copy_from_buf(&payload, msg, sizeof(hdr) + len);
    
    z_publisher_put(z_publisher_loan(&s->publisher), z_move(payload), NULL);
    
    g_free(msg);
    return len;
}

static void zenoh_chr_parse(QemuOpts *opts, ChardevBackend *backend, Error **errp)
{
    const char *node = qemu_opt_get(opts, "node");
    const char *router = qemu_opt_get(opts, "router");

    if (!node) {
        error_setg(errp, "chardev: zenoh: 'node' is required");
        return;
    }

    ChardevZenohOptions *zenoh_opts = g_new0(ChardevZenohOptions, 1);
    zenoh_opts->node = g_strdup(node);
    if (router) {
        zenoh_opts->router = g_strdup(router);
    }

    backend->type = CHARDEV_BACKEND_KIND_ZENOH;
    backend->u.zenoh.data = zenoh_opts;
    
    qemu_chr_parse_common(opts, qapi_ChardevZenohOptions_base(zenoh_opts));
}

static bool zenoh_chr_open(Chardev *chr, ChardevBackend *backend, Error **errp)
{
    ChardevZenoh *s = CHARDEV_ZENOH(chr);
    ChardevZenohOptions *opts = backend->u.zenoh.data;
    
    s->node_id = g_strdup(opts->node);
    s->router = g_strdup(opts->router ? opts->router : "");
    
    z_owned_config_t config;
    z_config_default(&config);
    if (s->router[0] != '\0') {
        zc_config_insert_json5(z_config_loan_mut(&config), "mode", "client");
        zc_config_insert_json5(z_config_loan_mut(&config), "connect/endpoints", s->router);
    }
    
    if (z_open(&s->session, z_move(config), NULL) != 0) {
        error_setg(errp, "Failed to open zenoh session");
        return false;
    }
    
    char *tx_topic = g_strdup_printf("virtmcu/uart/%s/tx", s->node_id);
    char *rx_topic = g_strdup_printf("virtmcu/uart/%s/rx", s->node_id);
    
    z_owned_keyexpr_t kexpr_tx;
    z_keyexpr_from_str(&kexpr_tx, tx_topic);
    z_declare_publisher(z_session_loan(&s->session), &s->publisher, z_keyexpr_loan(&kexpr_tx), NULL);
    z_keyexpr_drop(z_move(kexpr_tx));
    
    z_owned_closure_sample_t callback;
    z_closure_sample(&callback, on_zenoh_msg, NULL, s);
    
    z_owned_keyexpr_t kexpr_rx;
    z_keyexpr_from_str(&kexpr_rx, rx_topic);
    z_declare_subscriber(z_session_loan(&s->session), &s->subscriber, z_keyexpr_loan(&kexpr_rx), z_move(callback), NULL);
    z_keyexpr_drop(z_move(kexpr_rx));
    
    g_free(tx_topic);
    g_free(rx_topic);
    
    qemu_mutex_init(&s->mutex);
    s->rx_timer = timer_new_ns(QEMU_CLOCK_VIRTUAL, rx_timer_cb, s);
    
    return true;
}

static void zenoh_chr_finalize(Object *obj)
{
    ChardevZenoh *s = CHARDEV_ZENOH(obj);
    if (s->rx_timer) {
        timer_free(s->rx_timer);
        s->rx_timer = NULL;
    }
    
    z_publisher_drop(z_move(s->publisher));
    z_subscriber_drop(z_move(s->subscriber));
    z_close(z_session_loan_mut(&s->session), NULL);
    z_session_drop(z_move(s->session));
    
    qemu_mutex_destroy(&s->mutex);
    g_free(s->node_id);
    g_free(s->router);
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
