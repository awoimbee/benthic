/*
 * benthic — libdivecomputer wasm shim.
 *
 * Exposes a tiny C API to JavaScript: the descriptor table and a function that
 * parses a raw dump into the neutral RawDive JSON (see
 * crates/benthic-core/src/divecomputer.rs). The Rust web app deserialises it
 * and builds the domain model with the shared mapping.
 *
 * Device download is layered on top with dc_custom_open and Asyncify so the
 * synchronous libdivecomputer iostream can drive async WebSerial/WebBluetooth.
 */

#include <emscripten.h>

#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <libdivecomputer/context.h>
#include <libdivecomputer/custom.h>
#include <libdivecomputer/descriptor.h>
#include <libdivecomputer/device.h>
#include <libdivecomputer/iterator.h>
#include <libdivecomputer/parser.h>

/* ------------------------------------------------------------------ buffer */

typedef struct {
	char *data;
	size_t len;
	size_t cap;
} buf_t;

static void buf_need(buf_t *b, size_t extra)
{
	if (b->len + extra + 1 <= b->cap)
		return;
	size_t cap = b->cap ? b->cap : 256;
	while (cap < b->len + extra + 1)
		cap *= 2;
	b->data = realloc(b->data, cap);
	b->cap = cap;
}

static void buf_putn(buf_t *b, const char *s, size_t n)
{
	buf_need(b, n);
	memcpy(b->data + b->len, s, n);
	b->len += n;
	b->data[b->len] = '\0';
}

static void buf_put(buf_t *b, const char *s)
{
	buf_putn(b, s, strlen(s));
}

static void buf_printf(buf_t *b, const char *fmt, ...)
{
	char stack[128];
	va_list args;
	va_start(args, fmt);
	int n = vsnprintf(stack, sizeof(stack), fmt, args);
	va_end(args);
	if (n < 0)
		return;
	if ((size_t)n < sizeof(stack)) {
		buf_putn(b, stack, n);
		return;
	}
	buf_need(b, n);
	va_start(args, fmt);
	vsnprintf(b->data + b->len, n + 1, fmt, args);
	va_end(args);
	b->len += n;
}

static void buf_string(buf_t *b, const char *s)
{
	buf_put(b, "\"");
	for (const unsigned char *p = (const unsigned char *)s; *p; p++) {
		switch (*p) {
		case '"':  buf_put(b, "\\\""); break;
		case '\\': buf_put(b, "\\\\"); break;
		case '\n': buf_put(b, "\\n"); break;
		case '\r': buf_put(b, "\\r"); break;
		case '\t': buf_put(b, "\\t"); break;
		default:
			if (*p < 0x20)
				buf_printf(b, "\\u%04x", *p);
			else
				buf_putn(b, (const char *)p, 1);
		}
	}
	buf_put(b, "\"");
}

/* ------------------------------------------------------------- descriptors */

static void put_usage(buf_t *b, dc_usage_t usage)
{
	switch (usage) {
	case DC_USAGE_OXYGEN:       buf_put(b, "\"oxygen\""); break;
	case DC_USAGE_DILUENT:      buf_put(b, "\"diluent\""); break;
	case DC_USAGE_OPEN_CIRCUIT: buf_put(b, "\"open_circuit\""); break;
	default:                    buf_put(b, "\"none\""); break;
	}
}

static void put_divemode(buf_t *b, dc_divemode_t mode)
{
	switch (mode) {
	case DC_DIVEMODE_FREEDIVE: buf_put(b, "\"freedive\""); break;
	case DC_DIVEMODE_GAUGE:    buf_put(b, "\"gauge\""); break;
	case DC_DIVEMODE_CCR:      buf_put(b, "\"ccr\""); break;
	case DC_DIVEMODE_SCR:      buf_put(b, "\"scr\""); break;
	default:                   buf_put(b, "\"oc\""); break;
	}
}

static void put_deco_kind(buf_t *b, dc_deco_type_t kind)
{
	switch (kind) {
	case DC_DECO_NDL:        buf_put(b, "\"ndl\""); break;
	case DC_DECO_SAFETYSTOP: buf_put(b, "\"safety_stop\""); break;
	case DC_DECO_DEEPSTOP:   buf_put(b, "\"deep_stop\""); break;
	default:                 buf_put(b, "\"deco_stop\""); break;
	}
}

EMSCRIPTEN_KEEPALIVE
char *benthic_dc_descriptors(void)
{
	dc_context_t *context = NULL;
	dc_iterator_t *iterator = NULL;
	dc_descriptor_t *descriptor = NULL;
	buf_t b = {0};

	if (dc_context_new(&context) != DC_STATUS_SUCCESS)
		return NULL;
	if (dc_descriptor_iterator_new(&iterator, context) != DC_STATUS_SUCCESS) {
		dc_context_free(context);
		return NULL;
	}

	buf_put(&b, "[");
	int first = 1;
	while (dc_iterator_next(iterator, &descriptor) == DC_STATUS_SUCCESS) {
		if (!first)
			buf_put(&b, ",");
		first = 0;
		buf_put(&b, "{\"vendor\":");
		buf_string(&b, dc_descriptor_get_vendor(descriptor));
		buf_put(&b, ",\"product\":");
		buf_string(&b, dc_descriptor_get_product(descriptor));
		buf_printf(&b, ",\"transports\":%u", dc_descriptor_get_transports(descriptor));
		buf_put(&b, "}");
		dc_descriptor_free(descriptor);
	}
	buf_put(&b, "]");
	dc_iterator_free(iterator);
	dc_context_free(context);

	if (!b.data)
		b.data = strdup("[]");
	return b.data;
}

/* ------------------------------------------------------------------ parser */

typedef struct {
	buf_t *out;
	int first;
} sample_writer_t;

static void write_sample(dc_sample_type_t type, const dc_sample_value_t *value, void *userdata)
{
	sample_writer_t *w = userdata;

	if (type == DC_SAMPLE_GASMIX && value->gasmix == DC_GASMIX_UNKNOWN)
		return;
	switch (type) {
	case DC_SAMPLE_TIME:
	case DC_SAMPLE_DEPTH:
	case DC_SAMPLE_PRESSURE:
	case DC_SAMPLE_TEMPERATURE:
	case DC_SAMPLE_EVENT:
	case DC_SAMPLE_RBT:
	case DC_SAMPLE_HEARTBEAT:
	case DC_SAMPLE_BEARING:
	case DC_SAMPLE_SETPOINT:
	case DC_SAMPLE_PPO2:
	case DC_SAMPLE_CNS:
	case DC_SAMPLE_DECO:
	case DC_SAMPLE_GASMIX:
	case DC_SAMPLE_TTS:
		break;
	default:
		return;
	}

	if (!w->first)
		buf_put(w->out, ",");
	w->first = 0;

	switch (type) {
	case DC_SAMPLE_TIME:
		buf_printf(w->out, "{\"t\":\"time\",\"ms\":%u}", value->time);
		break;
	case DC_SAMPLE_DEPTH:
		buf_printf(w->out, "{\"t\":\"depth\",\"m\":%.17g}", value->depth);
		break;
	case DC_SAMPLE_PRESSURE:
		buf_printf(w->out, "{\"t\":\"pressure\",\"tank\":%u,\"bar\":%.17g}",
			value->pressure.tank, value->pressure.value);
		break;
	case DC_SAMPLE_TEMPERATURE:
		buf_printf(w->out, "{\"t\":\"temperature\",\"c\":%.17g}", value->temperature);
		break;
	case DC_SAMPLE_EVENT:
		buf_printf(w->out, "{\"t\":\"event\",\"kind\":%u,\"ms\":%u,\"flags\":%u,\"value\":%d",
			value->event.type, value->event.time, value->event.flags, (int)value->event.value);
		if (value->event.name) {
			buf_put(w->out, ",\"name\":");
			buf_string(w->out, value->event.name);
		}
		buf_put(w->out, "}");
		break;
	case DC_SAMPLE_RBT:
		buf_printf(w->out, "{\"t\":\"rbt\",\"s\":%u}", value->rbt);
		break;
	case DC_SAMPLE_HEARTBEAT:
		buf_printf(w->out, "{\"t\":\"heartbeat\",\"bpm\":%u}", value->heartbeat);
		break;
	case DC_SAMPLE_BEARING:
		buf_printf(w->out, "{\"t\":\"bearing\",\"deg\":%u}", value->bearing);
		break;
	case DC_SAMPLE_SETPOINT:
		buf_printf(w->out, "{\"t\":\"setpoint\",\"bar\":%.17g}", value->setpoint);
		break;
	case DC_SAMPLE_PPO2:
		buf_printf(w->out, "{\"t\":\"ppo2\",\"sensor\":%u,\"bar\":%.17g}",
			value->ppo2.sensor, value->ppo2.value);
		break;
	case DC_SAMPLE_CNS:
		buf_printf(w->out, "{\"t\":\"cns\",\"fraction\":%.17g}", value->cns);
		break;
	case DC_SAMPLE_DECO:
		buf_put(w->out, "{\"t\":\"deco\",\"kind\":");
		put_deco_kind(w->out, value->deco.type);
		buf_printf(w->out, ",\"s\":%u,\"m\":%.17g,\"tts\":%u}",
			value->deco.time, value->deco.depth, value->deco.tts);
		break;
	case DC_SAMPLE_GASMIX:
		buf_printf(w->out, "{\"t\":\"gasmix\",\"index\":%u}", value->gasmix);
		break;
	case DC_SAMPLE_TTS:
		buf_printf(w->out, "{\"t\":\"tts\",\"s\":%u}", value->time);
		break;
	default:
		break;
	}
}

static void put_u32_field(buf_t *b, const char *name, dc_parser_t *parser, dc_field_type_t field)
{
	unsigned int v = 0;
	if (dc_parser_get_field(parser, field, 0, &v) != DC_STATUS_SUCCESS)
		return;
	buf_printf(b, ",\"%s\":%u", name, v);
}

static void put_f64_field(buf_t *b, const char *name, dc_parser_t *parser, dc_field_type_t field)
{
	double v = 0;
	if (dc_parser_get_field(parser, field, 0, &v) != DC_STATUS_SUCCESS)
		return;
	buf_printf(b, ",\"%s\":%.17g", name, v);
}

EMSCRIPTEN_KEEPALIVE
char *benthic_dc_parse(const char *vendor, const char *product,
	const unsigned char *data, unsigned int size)
{
	dc_context_t *context = NULL;
	dc_iterator_t *iterator = NULL;
	dc_descriptor_t *descriptor = NULL;
	dc_descriptor_t *found = NULL;
	dc_parser_t *parser = NULL;
	buf_t b = {0};
	char *result = NULL;

	if (dc_context_new(&context) != DC_STATUS_SUCCESS)
		return NULL;
	if (dc_descriptor_iterator_new(&iterator, context) != DC_STATUS_SUCCESS)
		goto cleanup;

	while (dc_iterator_next(iterator, &descriptor) == DC_STATUS_SUCCESS) {
		const char *v = dc_descriptor_get_vendor(descriptor);
		const char *p = dc_descriptor_get_product(descriptor);
		if (v && p && strcmp(v, vendor) == 0 && strcmp(p, product) == 0) {
			found = descriptor;
			break;
		}
		dc_descriptor_free(descriptor);
	}
	dc_iterator_free(iterator);
	iterator = NULL;
	if (!found)
		goto cleanup;

	if (dc_parser_new2(&parser, context, found, data, size) != DC_STATUS_SUCCESS)
		goto cleanup;

	buf_put(&b, "{\"vendor\":");
	buf_string(&b, vendor);
	buf_put(&b, ",\"product\":");
	buf_string(&b, product);

	dc_datetime_t dt;
	if (dc_parser_get_datetime(parser, &dt) == DC_STATUS_SUCCESS) {
		buf_printf(&b,
			",\"datetime\":{\"year\":%d,\"month\":%u,\"day\":%u,\"hour\":%u,"
			"\"minute\":%u,\"second\":%u",
			dt.year, dt.month, dt.day, dt.hour, dt.minute, dt.second);
		if (dt.timezone != DC_TIMEZONE_NONE)
			buf_printf(&b, ",\"timezone\":%d", dt.timezone);
		buf_put(&b, "}");
	}

	put_u32_field(&b, "divetime", parser, DC_FIELD_DIVETIME);
	put_f64_field(&b, "max_depth", parser, DC_FIELD_MAXDEPTH);
	put_f64_field(&b, "mean_depth", parser, DC_FIELD_AVGDEPTH);
	put_f64_field(&b, "atmospheric", parser, DC_FIELD_ATMOSPHERIC);
	put_f64_field(&b, "temperature_surface", parser, DC_FIELD_TEMPERATURE_SURFACE);
	put_f64_field(&b, "temperature_min", parser, DC_FIELD_TEMPERATURE_MINIMUM);

	dc_salinity_t salinity;
	if (dc_parser_get_field(parser, DC_FIELD_SALINITY, 0, &salinity) == DC_STATUS_SUCCESS
		&& salinity.density > 0.0)
		buf_printf(&b, ",\"salinity_density\":%.17g", salinity.density);

	dc_divemode_t divemode;
	if (dc_parser_get_field(parser, DC_FIELD_DIVEMODE, 0, &divemode) == DC_STATUS_SUCCESS) {
		buf_put(&b, ",\"divemode\":");
		put_divemode(&b, divemode);
	}

	dc_decomodel_t decomodel;
	if (dc_parser_get_field(parser, DC_FIELD_DECOMODEL, 0, &decomodel) == DC_STATUS_SUCCESS
		&& (decomodel.params.gf.low > 0 || decomodel.params.gf.high > 0))
		buf_printf(&b, ",\"gf\":{\"low\":%u,\"high\":%u}",
			decomodel.params.gf.low, decomodel.params.gf.high);

	dc_event_devinfo_t devinfo;
	if (dc_parser_get_device_info(parser, &devinfo) == DC_STATUS_SUCCESS)
		buf_printf(&b,
			",\"info\":{\"model\":%u,\"firmware\":%u,\"serial\":%u,\"hw_id\":%u}",
			devinfo.model, devinfo.firmware, devinfo.serial, devinfo.hw_id);

	/* gas mixes */
	unsigned int count = 0;
	dc_parser_get_field(parser, DC_FIELD_GASMIX_COUNT, 0, &count);
	buf_put(&b, ",\"gasmixes\":[");
	for (unsigned int i = 0; i < count; i++) {
		dc_gasmix_t mix;
		if (dc_parser_get_field(parser, DC_FIELD_GASMIX, i, &mix) != DC_STATUS_SUCCESS)
			continue;
		if (i)
			buf_put(&b, ",");
		buf_printf(&b, "{\"oxygen\":%.17g,\"helium\":%.17g,\"usage\":", mix.oxygen, mix.helium);
		put_usage(&b, mix.usage);
		buf_put(&b, "}");
	}
	buf_put(&b, "]");

	/* tanks */
	count = 0;
	dc_parser_get_field(parser, DC_FIELD_TANK_COUNT, 0, &count);
	buf_put(&b, ",\"tanks\":[");
	for (unsigned int i = 0; i < count; i++) {
		dc_tank_t tank;
		if (dc_parser_get_field(parser, DC_FIELD_TANK, i, &tank) != DC_STATUS_SUCCESS)
			continue;
		if (i)
			buf_put(&b, ",");
		buf_put(&b, "{");
		if (tank.gasmix != DC_GASMIX_UNKNOWN)
			buf_printf(&b, "\"gasmix\":%u,", tank.gasmix);
		buf_printf(&b,
			"\"volume\":%.17g,\"workpressure\":%.17g,\"beginpressure\":%.17g,\"endpressure\":%.17g}",
			tank.volume, tank.workpressure, tank.beginpressure, tank.endpressure);
	}
	buf_put(&b, "]");

	/* samples */
	buf_put(&b, ",\"samples\":[");
	sample_writer_t writer = { .out = &b, .first = 1 };
	dc_parser_samples_foreach(parser, write_sample, &writer);
	buf_put(&b, "]}");

	result = b.data ? b.data : strdup("{}");
	b.data = NULL;

cleanup:
	if (parser)
		dc_parser_destroy(parser);
	if (found)
		dc_descriptor_free(found);
	if (iterator)
		dc_iterator_free(iterator);
	dc_context_free(context);
	free(b.data);
	return result;
}

EMSCRIPTEN_KEEPALIVE
void benthic_dc_free(char *ptr)
{
	free(ptr);
}
