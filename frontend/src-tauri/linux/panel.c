// Click-away for the GTK window containing the existing Tauri WebView.
// GDK owns the display and surface. All calls run on the GTK main thread.
#include <gtk/gtk.h>
#include <gdk/gdkwayland.h>
#include <wayland-client.h>
#include <string.h>
#include "focus-grab.h"

typedef struct {
    GtkWindow *window;
    gboolean armed;
    guint first_frame;
    guint pending_acquire;
    guint pending_close;
    struct hyprland_focus_grab_v1 *grab;
} Panel;

static void release(Panel *panel) {
    if (panel->first_frame) {
        gtk_widget_remove_tick_callback(GTK_WIDGET(panel->window), panel->first_frame);
        panel->first_frame = 0;
    }
    g_clear_handle_id(&panel->pending_acquire, g_source_remove);
    g_clear_handle_id(&panel->pending_close, g_source_remove);
    if (panel->grab) {
        hyprland_focus_grab_v1_destroy(panel->grab);
        panel->grab = NULL;
    }
}

static gboolean dismiss(gpointer data) {
    Panel *panel = data;
    panel->pending_close = 0;
    if (panel->armed && gtk_widget_get_mapped(GTK_WIDGET(panel->window)))
        gtk_window_close(panel->window);
    return G_SOURCE_REMOVE;
}

static void cleared(void *data, struct hyprland_focus_grab_v1 *grab) {
    Panel *panel = data;
    if (panel->grab == grab && !panel->pending_close)
        panel->pending_close = g_idle_add(dismiss, panel);
}

static const struct hyprland_focus_grab_v1_listener grab_listener = {cleared};

static void global(void *data, struct wl_registry *registry, uint32_t name,
                   const char *interface, uint32_t version) {
    struct hyprland_focus_grab_manager_v1 **manager = data;
    if (!*manager && version >= 1 && !strcmp(interface, hyprland_focus_grab_manager_v1_interface.name))
        *manager = wl_registry_bind(registry, name, &hyprland_focus_grab_manager_v1_interface, 1);
}
static void removed(void *data, struct wl_registry *registry, uint32_t name) {}
static const struct wl_registry_listener registry_listener = {global, removed};

static void acquire(GtkWidget *widget, Panel *panel) {
    if (!panel->armed || panel->grab || !gtk_widget_get_mapped(widget)) return;
    GdkDisplay *display = gtk_widget_get_display(widget);
    if (!GDK_IS_WAYLAND_DISPLAY(display)) return;
    struct wl_surface *surface = gdk_wayland_window_get_wl_surface(gtk_widget_get_window(widget));
    if (!surface) return;

    // Dispatch only our registry during discovery; GDK dispatches the grab thereafter.
    struct wl_display *wl = gdk_wayland_display_get_wl_display(display);
    struct wl_event_queue *queue = wl_display_create_queue(wl);
    if (!queue) return;
    struct wl_registry *registry = wl_display_get_registry(wl);
    if (!registry) {
        wl_event_queue_destroy(queue);
        return;
    }
    wl_proxy_set_queue((struct wl_proxy *)registry, queue);
    struct hyprland_focus_grab_manager_v1 *manager = NULL;
    wl_registry_add_listener(registry, &registry_listener, &manager);
    if (wl_display_roundtrip_queue(wl, queue) >= 0 && manager) {
        wl_proxy_set_queue((struct wl_proxy *)manager, NULL);
        panel->grab = hyprland_focus_grab_manager_v1_create_grab(manager);
        if (panel->grab) {
            hyprland_focus_grab_v1_add_listener(panel->grab, &grab_listener, panel);
            hyprland_focus_grab_v1_add_surface(panel->grab, surface);
            hyprland_focus_grab_v1_commit(panel->grab);
        }
    }
    if (manager) hyprland_focus_grab_manager_v1_destroy(manager);
    wl_registry_destroy(registry);
    wl_event_queue_destroy(queue);
}

static gboolean acquire_after_paint(gpointer data) {
    Panel *panel = data;
    panel->pending_acquire = 0;
    acquire(GTK_WIDGET(panel->window), panel);
    return G_SOURCE_REMOVE;
}

static gboolean first_frame(GtkWidget *widget, GdkFrameClock *clock, gpointer data) {
    Panel *panel = data;
    panel->first_frame = 0;
    panel->pending_acquire = g_idle_add(acquire_after_paint, panel);
    return G_SOURCE_REMOVE;
}

static void mapped(GtkWidget *widget, Panel *panel) {
    // Let GTK commit the first buffer before the compositor grabs the surface.
    if (panel->armed && !panel->first_frame && !panel->pending_acquire)
        panel->first_frame = gtk_widget_add_tick_callback(widget, first_frame, panel, NULL);
}

static void unmapped(GtkWidget *widget, Panel *panel) { release(panel); }
static gboolean focus_out(GtkWidget *widget, GdkEventFocus *event, Panel *panel) {
    // Fallback for desktops without the compositor's click-away protocol.
    if (panel->armed && !panel->grab && !panel->first_frame &&
        !panel->pending_acquire && !panel->pending_close)
        panel->pending_close = g_idle_add(dismiss, panel);
    return FALSE;
}
static void free_panel(gpointer data) { release(data); g_free(data); }

void ayla_panel_click_away(GtkWindow *window, gboolean armed) {
    Panel *panel = g_object_get_data(G_OBJECT(window), "ayla-quick-panel");
    if (!panel && !armed) return;
    if (!panel) {
        panel = g_new0(Panel, 1);
        panel->window = window;
        g_object_set_data_full(G_OBJECT(window), "ayla-quick-panel", panel, free_panel);
        g_signal_connect_after(window, "map", G_CALLBACK(mapped), panel);
        g_signal_connect(window, "unmap", G_CALLBACK(unmapped), panel);
        g_signal_connect(window, "focus-out-event", G_CALLBACK(focus_out), panel);
    }
    panel->armed = armed;
    if (armed && gtk_widget_get_mapped(GTK_WIDGET(window))) mapped(GTK_WIDGET(window), panel);
    else release(panel);
}
