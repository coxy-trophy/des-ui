// SPDX-License-Identifier: MIT
package io.github.coxytrophy.desconnect.companion;

import android.app.Activity;
import android.content.Context;
import android.content.Intent;
import android.content.SharedPreferences;
import android.content.res.ColorStateList;
import android.graphics.Typeface;
import android.graphics.drawable.GradientDrawable;
import android.graphics.drawable.RippleDrawable;
import android.graphics.drawable.StateListDrawable;
import android.media.projection.MediaProjectionConfig;
import android.media.projection.MediaProjectionManager;
import android.os.Build;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.provider.Settings;
import android.text.InputFilter;
import android.text.InputType;
import android.view.Gravity;
import android.view.View;
import android.view.WindowInsets;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputMethodManager;
import android.widget.Button;
import android.widget.EditText;
import android.widget.FrameLayout;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;

import java.util.Collections;
import java.util.List;
import java.util.Objects;

public final class MainActivity extends Activity {
    private static final int DEFAULT_PORT = 1716;
    private static final int CAST_CONSENT = 43;
    private static final int CAST_SETTINGS = 44;
    private static final int TOUCH_TARGET_DP = 48;
    private static final int CONTROL_RADIUS_DP = 6;
    private static final int PANEL_RADIUS_DP = 10;
    private static final String PREFS = "last_endpoint";
    private static final String HOST = "host";
    private static final String PORT = "port";

    private final Handler main = new Handler(Looper.getMainLooper());
    private Link link;
    private DesktopDiscovery discovery;
    private Link.View state;
    private boolean visible;
    private int linkGeneration;
    private int discoveryGeneration;
    private boolean discovering;
    private List<DesktopDiscovery.Desktop> desktops = Collections.emptyList();
    private final Runnable discoveryWait = () -> {
        discovering = false;
        renderDiscovery();
    };
    private String lastHost = "";
    private int lastPort = DEFAULT_PORT;
    private boolean endpointSaved;
    private Link.Phase pendingPhase;
    private long pendingAttempt;
    private boolean approvedOutgoing;
    private boolean confirmUnpair;
    private boolean showingSetup = true;

    private Typeface display;
    private Typeface strong;
    private Typeface mono;
    private ScrollView scroller;
    private View lamp;
    private TextView status;
    private TextView desktopName;
    private TextView detail;
    private LinearLayout verification;
    private TextView code;
    private TextView verificationHelp;
    private TextView unpairHelp;
    private LinearLayout setup;
    private LinearLayout nearbyList;
    private TextView nearbyHelp;
    private EditText address;
    private EditText port;
    private Button reconnectButton;
    private Button disconnectButton;
    private Button pairButton;
    private Button acceptButton;
    private Button rejectButton;
    private Button pingButton;
    private Button unpairButton;
    private Button forgetButton;
    private Button confirmButton;
    private Button cancelButton;
    private LinearLayout approvalActions;
    private Link.View shownApproval;
    private LinearLayout pairedActions;
    private LinearLayout confirmActions;
    private LinearLayout castPanel;
    private TextView castDetail;
    private Button castView;
    private Button castControl;
    private Button castStop;
    private CastSession.Token castConsent;
    private boolean castConsentControl;
    private boolean castSettings;

    @Override protected void onCreate(Bundle saved) {
        super.onCreate(saved);
        display = Typeface.createFromAsset(getAssets(), "ChakraPetch-Medium.ttf");
        strong = Typeface.createFromAsset(getAssets(), "ChakraPetch-SemiBold.ttf");
        mono = Typeface.createFromAsset(getAssets(), "JetBrainsMono.ttf");

        SharedPreferences preferences = getSharedPreferences(PREFS, MODE_PRIVATE);
        lastHost = preferences.getString(HOST, "");
        lastPort = preferences.getInt(PORT, DEFAULT_PORT);
        buildScreen();
        address.setText(lastHost);
        port.setText(String.valueOf(lastPort));
        render();
    }

    @Override protected void onStart() {
        super.onStart();
        visible = true;
        if (link == null) prepareLink();
        link.cast.foreground(true);
        render();
        final int generation = ++discoveryGeneration;
        discovery = new DesktopDiscovery(found -> main.post(() -> {
            if (!visible || generation != discoveryGeneration) return;
            desktops = found;
            if (!found.isEmpty()) {
                discovering = false;
                main.removeCallbacks(discoveryWait);
            }
            renderDiscovery();
        }));
        desktops = Collections.emptyList();
        discovering = true;
        renderDiscovery();
        main.postDelayed(discoveryWait, 4500);
        discovery.start();
    }

    @Override protected void onResume() {
        super.onResume();
        render();
    }

    @Override protected void onStop() {
        visible = false;
        ++discoveryGeneration;
        main.removeCallbacks(discoveryWait);
        if (discovery != null) {
            discovery.close();
            discovery = null;
        }
        if (link != null) link.cast.foreground(false);
        if (!retainCastLink()) {
            closeLink();
            state = null;
            render();
        }
        super.onStop();
    }

    @Override protected void onDestroy() {
        closeLink();
        super.onDestroy();
    }

    private boolean retainCastLink() {
        CastSession.View cast = link == null ? null : link.cast.view();
        return cast != null && (cast.phase != CastSession.Phase.REQUESTED || castSettings);
    }

    private void castChanged(int generation) {
        if (generation != linkGeneration) return;
        if (!visible && !retainCastLink()) {
            closeLink();
            state = null;
        }
        render();
    }

    private void prepareLink() {
        closeLink();
        final int generation = linkGeneration;
        link = new Link(getApplicationContext(), update -> main.post(() -> {
            if (generation != linkGeneration || !visible && !retainCastLink()) return;
            if (state != null && state.phase == Link.Phase.CONNECTING
                    && update.phase == Link.Phase.DISCONNECTED && update.detail.isEmpty()) return;
            if (pendingPhase != null && (update.retryAction || update.phase != pendingPhase
                    || update.attempt != pendingAttempt || isOffline(pendingPhase)
                    && (state == null || !Objects.equals(update.detail, state.detail)))) {
                pendingPhase = null;
                approvedOutgoing = false;
            }
            if (update.phase != Link.Phase.PAIRED && !(isOffline(update.phase) && update.savedDesktop)) {
                confirmUnpair = false;
            }
            if (!endpointSaved && !lastHost.isEmpty()
                    && (update.phase == Link.Phase.PAIRED || update.phase == Link.Phase.UNPAIRED
                        || update.phase == Link.Phase.INCOMING || update.phase == Link.Phase.OUTGOING)) {
                getSharedPreferences(PREFS, MODE_PRIVATE).edit().putString(HOST, lastHost).putInt(PORT, lastPort).apply();
                endpointSaved = true;
            }
            state = update;
            render();
        }), () -> main.post(() -> castChanged(generation)));
        link.cast.foreground(visible);
    }

    private void closeLink() {
        ++linkGeneration;
        pendingPhase = null;
        approvedOutgoing = false;
        confirmUnpair = false;
        castConsent = null;
        castSettings = false;
        if (link != null) {
            link.close();
            link = null;
        }
    }

    private void connectManually() {
        String host = address.getText().toString().trim();
        if (host.isEmpty()) {
            address.setError(getText(R.string.invalid_address));
            address.requestFocus();
            return;
        }
        int value;
        try {
            value = Integer.parseInt(port.getText().toString());
        } catch (NumberFormatException invalid) {
            value = 0;
        }
        if (value < 1 || value > 65535) {
            port.setError(getText(R.string.invalid_port));
            port.requestFocus();
            return;
        }
        connectTo(host, value);
    }

    private void connectTo(String host, int value) {
        if (!visible || !showingSetup) return;
        lastHost = host;
        lastPort = value;
        address.setText(host);
        port.setText(String.valueOf(value));
        endpointSaved = false;
        InputMethodManager input = (InputMethodManager) getSystemService(Context.INPUT_METHOD_SERVICE);
        input.hideSoftInputFromWindow(scroller.getWindowToken(), 0);
        scroller.requestFocus();
        scroller.scrollTo(0, 0);

        prepareLink();
        state = new Link.View(Link.Phase.CONNECTING, "", "", "", 0);
        render();
        link.connect(host, value);
    }

    private void disconnect() {
        if (!visible) return;
        boolean saved = state != null && state.savedDesktop;
        prepareLink();
        state = new Link.View(Link.Phase.DISCONNECTED, "", "", "", 0, saved);
        render();
        reconnectButton.requestFocus();
    }

    private void requestPairing() {
        if (link == null || state == null || state.phase != Link.Phase.UNPAIRED || pendingPhase != null) return;
        markPending();
        link.pair();
    }

    private void decidePairing(Link.View shown, int generation, boolean approve) {
        if (generation != linkGeneration || !sameApproval(shown, state)) return;
        if (link == null || state == null || state.attempt == 0) return;
        if (pendingPhase != null && (approve || !approvedOutgoing)) return;
        if (state.phase != Link.Phase.INCOMING && state.phase != Link.Phase.OUTGOING) return;
        if (approve && !hasCode()) return;
        approvedOutgoing = approve && state.phase == Link.Phase.OUTGOING;
        markPending();
        if (approve) link.accept(pendingAttempt);
        else link.reject(pendingAttempt);
    }

    private void markPending() {
        pendingPhase = state.phase;
        pendingAttempt = state.attempt;
        render();
    }

    private boolean hasCode() {
        return state != null && state.code != null && state.code.matches("[0-9a-fA-F]{8}");
    }

    private void buildScreen() {
        FrameLayout root = new FrameLayout(this);
        root.setBackgroundColor(color(R.color.ground));
        scroller = new ScrollView(this);
        scroller.setFillViewport(true);
        scroller.setFocusableInTouchMode(true);
        root.addView(scroller, new FrameLayout.LayoutParams(-1, -1));
        if (Build.VERSION.SDK_INT >= 30) {
            getWindow().setDecorFitsSystemWindows(false);
            root.setOnApplyWindowInsetsListener((view, insets) -> {
                android.graphics.Insets system = insets.getInsets(WindowInsets.Type.systemBars()
                        | WindowInsets.Type.displayCutout() | WindowInsets.Type.ime());
                view.setPadding(system.left, system.top, system.right, system.bottom);
                return insets;
            });
        }

        // Fill the phone width, but keep the instrument readable on wider displays.
        FrameLayout width = new FrameLayout(this) {
            @Override protected void onMeasure(int widthSpec, int heightSpec) {
                FrameLayout.LayoutParams child = (FrameLayout.LayoutParams) getChildAt(0).getLayoutParams();
                child.width = Math.min(View.MeasureSpec.getSize(widthSpec), dp(544));
                super.onMeasure(widthSpec, heightSpec);
            }
        };
        scroller.addView(width, new ScrollView.LayoutParams(-1, -2));
        LinearLayout content = column();
        content.setPadding(dp(20), dp(16), dp(20), dp(24));
        width.addView(content, new FrameLayout.LayoutParams(-1, -2, Gravity.TOP | Gravity.CENTER_HORIZONTAL));

        LinearLayout brand = row();
        brand.setGravity(Gravity.CENTER_VERTICAL);
        ImageView symbol = new ImageView(this);
        symbol.setImageResource(R.drawable.ic_des_connect);
        symbol.setImportantForAccessibility(View.IMPORTANT_FOR_ACCESSIBILITY_NO);
        brand.addView(symbol, new LinearLayout.LayoutParams(dp(48), dp(48)));

        LinearLayout wordmark = column();
        LinearLayout.LayoutParams wordmarkParams = new LinearLayout.LayoutParams(0, -2, 1);
        wordmarkParams.setMarginStart(dp(10));
        brand.addView(wordmark, wordmarkParams);
        TextView title = text(R.string.wordmark, strong, 23, R.color.text);
        if (Build.VERSION.SDK_INT >= 28) title.setAccessibilityHeading(true);
        wordmark.addView(title);
        wordmark.addView(text(R.string.masthead_subtitle, display, 14, R.color.secondary));
        TextView preview = eyebrow(R.string.preview, R.color.accent);
        preview.setPadding(dp(8), dp(5), dp(8), dp(5));
        preview.setBackground(shape(color(R.color.accent_track), color(R.color.line_strong), CONTROL_RADIUS_DP));
        brand.addView(preview, new LinearLayout.LayoutParams(-2, -2));
        add(content, brand, 0);

        LinearLayout linkPanel = panel();
        add(content, linkPanel, 20);
        add(linkPanel, eyebrow(R.string.local_link, R.color.secondary), 0);

        LinearLayout statusRow = row();
        statusRow.setGravity(Gravity.CENTER_VERTICAL);
        lamp = new View(this);
        lamp.setImportantForAccessibility(View.IMPORTANT_FOR_ACCESSIBILITY_NO);
        statusRow.addView(lamp, new LinearLayout.LayoutParams(dp(8), dp(8)));
        status = text(R.string.status_idle, mono, 13, R.color.secondary);
        status.setId(R.id.link_status);
        status.setAccessibilityLiveRegion(View.ACCESSIBILITY_LIVE_REGION_POLITE);
        LinearLayout.LayoutParams statusParams = new LinearLayout.LayoutParams(-1, -2);
        statusParams.setMarginStart(dp(8));
        statusRow.addView(status, statusParams);
        add(linkPanel, statusRow, 10);

        desktopName = text(R.string.idle_title, strong, 25, R.color.text);
        desktopName.setId(R.id.desktop_name);
        if (Build.VERSION.SDK_INT >= 28) desktopName.setAccessibilityHeading(true);
        add(linkPanel, desktopName, 5);
        detail = text(R.string.idle_detail, display, 16, R.color.secondary);
        detail.setId(R.id.link_detail);
        detail.setAccessibilityLiveRegion(View.ACCESSIBILITY_LIVE_REGION_POLITE);
        add(linkPanel, detail, 4);

        verification = column();
        verification.setPadding(dp(12), dp(12), dp(12), dp(12));
        verification.setBackground(shape(color(R.color.track), color(R.color.line_strong), CONTROL_RADIUS_DP));
        add(verification, eyebrow(R.string.verification_label, R.color.accent), 0);
        code = text(0, mono, 27, R.color.text);
        code.setId(R.id.verification_code);
        code.setLetterSpacing(0.08f);
        add(verification, code, 4);
        verificationHelp = text(R.string.verification_instruction, display, 15, R.color.secondary);
        add(verification, verificationHelp, 4);
        add(linkPanel, verification, 12);

        pairButton = button(R.id.pair, R.string.pair_action, true, false, this::requestPairing);
        add(linkPanel, pairButton, 12);
        approvalActions = row();
        approvalActions.setBaselineAligned(false);
        add(linkPanel, approvalActions, 12);
        pingButton = button(R.id.ping, R.string.ping_action, true, false, () -> {
            if (link != null && state != null && state.phase == Link.Phase.PAIRED && pendingPhase == null) link.ping();
        });
        unpairButton = button(R.id.unpair, R.string.unpair_action, false, false, this::beginUnpair);
        forgetButton = button(R.id.forget, R.string.forget_action, false, false, this::beginUnpair);
        pairedActions = buttons(pingButton, unpairButton);
        add(linkPanel, pairedActions, 12);
        unpairHelp = text(R.string.unpair_detail, display, 16, R.color.caution);
        unpairHelp.setAccessibilityLiveRegion(View.ACCESSIBILITY_LIVE_REGION_POLITE);
        add(linkPanel, unpairHelp, 12);
        cancelButton = button(R.id.cancel_unpair, R.string.cancel_action, false, false, () -> {
            confirmUnpair = false;
            render();
            if (state != null) (isOffline(state.phase) ? forgetButton : unpairButton).requestFocus();
        });
        confirmButton = button(R.id.confirm_unpair, R.string.confirm_unpair_action, false, true, () -> {
            if (link == null || !confirmUnpair || !canUnpair()) return;
            confirmUnpair = false;
            markPending();
            link.unpair();
        });
        confirmActions = buttons(cancelButton, confirmButton);
        add(linkPanel, confirmActions, 8);
        disconnectButton = button(R.id.disconnect, R.string.disconnect_action, false, false, this::disconnect);
        add(linkPanel, disconnectButton, 8);
        reconnectButton = button(R.id.reconnect, R.string.reconnect_action, true, false, () -> connectTo(lastHost, lastPort));
        add(linkPanel, reconnectButton, 12);
        add(linkPanel, forgetButton, 8);

        castPanel = panel();
        add(castPanel, eyebrow(R.string.cast_title, R.color.accent), 0);
        castDetail = text(R.string.cast_requested, display, 16, R.color.secondary);
        castDetail.setAccessibilityLiveRegion(View.ACCESSIBILITY_LIVE_REGION_POLITE);
        add(castPanel, castDetail, 6);
        castView = button(R.id.cast_view, R.string.cast_view_action, true, false, () -> requestCast(false));
        add(castPanel, castView, 12);
        castControl = button(R.id.cast_control, R.string.cast_control_action, false, false, () -> requestCast(true));
        add(castPanel, castControl, 8);
        castStop = button(R.id.cast_stop, R.string.cast_stop, false, true, this::stopCast);
        add(castPanel, castStop, 8);
        add(content, castPanel, 12);

        setup = column();
        add(content, setup, 12);
        LinearLayout nearby = panel();
        add(nearby, heading(R.string.nearby_title), 0);
        nearbyHelp = text(R.string.nearby_scanning, display, 15, R.color.secondary);
        nearbyHelp.setMinLines(2);
        add(nearby, nearbyHelp, 5);
        nearbyList = column();
        nearbyList.setId(R.id.nearby_desktops);
        add(nearby, nearbyList, 0);
        add(setup, nearby, 0);

        LinearLayout manual = panel();
        add(manual, heading(R.string.manual_title), 0);
        add(manual, text(R.string.manual_intro, display, 15, R.color.secondary), 4);
        LinearLayout fields = row();
        fields.setBaselineAligned(false);
        fields.setGravity(Gravity.BOTTOM);
        address = field(R.id.desktop_address, R.string.desktop_address_hint, false);
        port = field(R.id.desktop_port, 0, true);
        LinearLayout.LayoutParams hostParams = new LinearLayout.LayoutParams(0, -2, 2);
        hostParams.setMarginEnd(dp(8));
        fields.addView(labeledField(R.string.desktop_address_label, address), hostParams);
        fields.addView(labeledField(R.string.desktop_port_label, port), new LinearLayout.LayoutParams(0, -2, 1));
        address.setImeOptions(EditorInfo.IME_ACTION_NEXT);
        address.setNextFocusForwardId(R.id.desktop_port);
        port.setImeOptions(EditorInfo.IME_ACTION_DONE);
        port.setOnEditorActionListener((view, action, event) -> {
            if (action != EditorInfo.IME_ACTION_DONE) return false;
            connectManually();
            return true;
        });
        add(manual, fields, 12);
        add(manual, button(R.id.connect, R.string.connect_action, true, false, this::connectManually), 10);
        add(setup, manual, 12);

        setContentView(root);
        scroller.requestFocus();
    }

    private void render() {
        Link.Phase phase = state != null ? state.phase : null;
        boolean active = phase != null && !isOffline(phase);
        boolean pending = pendingPhase != null;
        boolean approval = phase == Link.Phase.INCOMING || phase == Link.Phase.OUTGOING;
        boolean paired = phase == Link.Phase.PAIRED;
        boolean validCode = hasCode();
        int stateLabel = R.string.status_idle;
        int stateDetail = R.string.idle_detail;
        int signal = R.color.secondary;
        if (phase != null) switch (phase) {
            case CONNECTING:
                stateLabel = R.string.status_connecting;
                stateDetail = R.string.connecting_detail;
                signal = R.color.accent;
                break;
            case UNPAIRED:
                stateLabel = R.string.status_unpaired;
                stateDetail = R.string.unpaired_detail;
                break;
            case INCOMING:
                stateLabel = R.string.status_incoming;
                stateDetail = R.string.incoming_detail;
                signal = R.color.caution;
                break;
            case OUTGOING:
                stateLabel = R.string.status_outgoing;
                stateDetail = R.string.outgoing_detail;
                signal = R.color.caution;
                break;
            case PAIRED:
                stateLabel = R.string.status_paired;
                stateDetail = R.string.paired_detail;
                signal = R.color.accent;
                break;
            case DISCONNECTED:
                stateLabel = R.string.status_disconnected;
                stateDetail = R.string.disconnected_detail;
                break;
            case ERROR:
                stateLabel = R.string.status_error;
                stateDetail = R.string.failed_detail;
                signal = R.color.caution;
                break;
        }
        if (pending) {
            if (approvedOutgoing) {
                stateLabel = R.string.status_codes_confirmed;
                stateDetail = R.string.outgoing_approved_detail;
            } else if (phase == Link.Phase.PAIRED || isOffline(phase)) {
                stateLabel = R.string.status_removing_trust;
                stateDetail = R.string.removing_trust_detail;
            } else if (phase == Link.Phase.UNPAIRED) {
                stateLabel = R.string.status_requesting_pair;
                stateDetail = R.string.requesting_pair_detail;
            } else {
                stateLabel = R.string.status_responding;
                stateDetail = R.string.responding_detail;
            }
        }

        updateText(status, getText(stateLabel));
        status.setTextColor(color(signal));
        lamp.setBackground(shape(color(signal), 0, 4));
        updateText(desktopName, state != null && state.desktop != null && !state.desktop.isEmpty()
                ? state.desktop : state != null && !lastHost.isEmpty() ? lastHost : getText(R.string.idle_title));
        updateText(detail, !pending && state != null && state.detail != null && !state.detail.isEmpty()
                ? state.detail : getText(stateDetail));

        show(verification, approval);
        show(code, validCode);
        updateText(code, approval && validCode ? state.code : "");
        int instructions = phase == Link.Phase.OUTGOING && !approvedOutgoing
                ? R.string.outgoing_verification_instruction : R.string.verification_instruction;
        updateText(verificationHelp, getText(validCode ? instructions : R.string.verification_unavailable));
        show(pairButton, phase == Link.Phase.UNPAIRED);
        pairButton.setEnabled(!pending);
        renderApproval(approval ? state : null, pending, validCode);
        show(pairedActions, paired && !confirmUnpair);
        pingButton.setEnabled(!pending);
        unpairButton.setEnabled(!pending);
        show(unpairHelp, confirmUnpair);
        updateText(unpairHelp, getText(active ? R.string.unpair_detail : R.string.forget_detail));
        show(confirmActions, confirmUnpair);
        updateText(confirmButton, getText(active ? R.string.confirm_unpair_action : R.string.forget_confirm_action));
        show(disconnectButton, active && !confirmUnpair);
        show(reconnectButton, !active && !lastHost.isEmpty() && !confirmUnpair && !pending);
        show(forgetButton, !active && state != null && state.savedDesktop && !confirmUnpair && !pending);
        showingSetup = !active && !confirmUnpair && !pending;
        show(setup, showingSetup);
        renderCast();
    }

    private void renderCast() {
        CastSession.View cast = link == null ? null : link.cast.view();
        boolean requested = cast != null && cast.phase == CastSession.Phase.REQUESTED;
        show(castPanel, cast != null);
        if (cast == null) return;
        boolean available = CastControlService.available();
        show(castView, requested);
        show(castControl, requested);
        castView.setEnabled(castConsent == null && !castSettings);
        castControl.setEnabled(castConsent == null && !castSettings);
        updateText(castDetail, getText(requested ? R.string.cast_requested
                : cast.phase == CastSession.Phase.CONSENT ? R.string.cast_waiting
                : cast.control ? R.string.cast_live_control : R.string.cast_live_view));
        updateText(castControl, getText(available ? R.string.cast_control_action : R.string.cast_enable_control));
        updateText(castStop, getText(requested ? R.string.cast_decline : R.string.cast_stop));
    }

    private void requestCast(boolean control) {
        if (!visible || link == null || castConsent != null || castSettings) return;
        CastSession.View cast = link.cast.view();
        if (cast == null || cast.phase != CastSession.Phase.REQUESTED) return;
        if (control && !CastControlService.available()) {
            castSettings = true;
            renderCast();
            try {
                startActivityForResult(new Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS), CAST_SETTINGS);
            } catch (RuntimeException unavailable) {
                castSettings = false;
                renderCast();
            }
            return;
        }
        if (!link.cast.approve(cast.token, control)) return;
        castConsent = cast.token;
        castConsentControl = control;
        renderCast();
        try {
            MediaProjectionManager manager = getSystemService(MediaProjectionManager.class);
            Intent request = control && Build.VERSION.SDK_INT >= 34
                    ? manager.createScreenCaptureIntent(MediaProjectionConfig.createConfigForDefaultDisplay())
                    : manager.createScreenCaptureIntent();
            startActivityForResult(request, CAST_CONSENT);
        } catch (RuntimeException unavailable) {
            castConsent = null;
            link.cast.stop(cast.token);
        }
    }

    private void stopCast() {
        if (link != null) link.cast.stop();
    }

    @Override protected void onActivityResult(int request, int result, Intent data) {
        super.onActivityResult(request, result, data);
        if (request == CAST_SETTINGS) {
            castSettings = false;
            renderCast();
            return;
        }
        if (request != CAST_CONSENT) return;
        CastSession.Token token = castConsent;
        castConsent = null;
        if (link != null && token != null && (result != RESULT_OK
                || !CastCaptureService.launch(this, link.cast, token, data, castConsentControl))) {
            link.cast.stop(token);
        }
        renderCast();
    }

    private void beginUnpair() {
        if (!visible || !canUnpair()) return;
        confirmUnpair = true;
        render();
        cancelButton.requestFocus();
    }

    private boolean canUnpair() {
        return state != null && pendingPhase == null && (state.phase == Link.Phase.PAIRED
                || isOffline(state.phase) && state.savedDesktop);
    }

    private static boolean isOffline(Link.Phase phase) {
        return phase == Link.Phase.ERROR || phase == Link.Phase.DISCONNECTED;
    }

    private void renderApproval(Link.View current, boolean pending, boolean validCode) {
        if (!sameApproval(shownApproval, current)) {
            // A gesture started on a previous attempt must never approve the next code.
            approvalActions.removeAllViews();
            shownApproval = current;
            if (current != null) {
                final int generation = linkGeneration;
                acceptButton = button(R.id.accept, current.phase == Link.Phase.OUTGOING
                        ? R.string.codes_match_action : R.string.accept_action, true, false,
                        () -> decidePairing(current, generation, true));
                rejectButton = button(R.id.reject, R.string.reject_action, false, false,
                        () -> decidePairing(current, generation, false));
                addButtons(approvalActions, acceptButton, rejectButton);
            }
        }
        if (current != null) {
            acceptButton.setEnabled(validCode && !pending && current.attempt != 0);
            rejectButton.setEnabled((!pending || approvedOutgoing) && current.attempt != 0);
        }
        show(approvalActions, current != null);
    }

    private static boolean sameApproval(Link.View first, Link.View second) {
        return first == second || first != null && second != null
                && first.phase == second.phase && first.attempt == second.attempt
                && Objects.equals(first.code, second.code);
    }

    private void renderDiscovery() {
        updateText(nearbyHelp, getText(desktops.isEmpty()
                ? discovering ? R.string.nearby_scanning : R.string.nearby_empty
                : R.string.nearby_hint));
        // Keep unchanged rows (and keyboard/accessibility focus) across broadcasts.
        for (int i = 0; i < desktops.size(); ++i) {
            DesktopDiscovery.Desktop desktop = desktops.get(i);
            String endpoint = (desktop.host.contains(":") ? "[" + desktop.host + "]" : desktop.host) + ":" + desktop.port;
            String label = desktop.name + "\n" + endpoint;
            Button existing = (Button) nearbyList.getChildAt(i);
            if (existing != null && label.contentEquals(existing.getText())) continue;
            Button device = button(R.id.discovered_desktop, 0, false, false, () -> connectTo(desktop.host, desktop.port));
            device.setGravity(Gravity.CENTER_VERTICAL | Gravity.START);
            device.setText(label);
            device.setContentDescription(getString(R.string.discovered_desktop, desktop.name, endpoint));
            LinearLayout.LayoutParams params = new LinearLayout.LayoutParams(-1, -2);
            params.topMargin = dp(8);
            if (existing != null) nearbyList.removeViewAt(i);
            nearbyList.addView(device, i, params);
        }
        while (nearbyList.getChildCount() > desktops.size()) nearbyList.removeViewAt(desktops.size());
    }

    private static void show(View view, boolean visible) {
        view.setVisibility(visible ? View.VISIBLE : View.GONE);
    }

    private static void updateText(TextView view, CharSequence text) {
        if (!text.toString().contentEquals(view.getText())) view.setText(text);
    }

    private LinearLayout panel() {
        LinearLayout panel = column();
        panel.setPadding(dp(14), dp(14), dp(14), dp(14));
        panel.setBackground(shape(color(R.color.panel), color(R.color.line), PANEL_RADIUS_DP));
        return panel;
    }

    private LinearLayout column() {
        LinearLayout column = new LinearLayout(this);
        column.setOrientation(LinearLayout.VERTICAL);
        return column;
    }

    private LinearLayout row() {
        LinearLayout row = new LinearLayout(this);
        row.setOrientation(LinearLayout.HORIZONTAL);
        return row;
    }

    private void add(LinearLayout column, View child, int topMargin) {
        LinearLayout.LayoutParams params = new LinearLayout.LayoutParams(-1, -2);
        params.topMargin = dp(topMargin);
        column.addView(child, params);
    }

    private LinearLayout buttons(Button first, Button second) {
        LinearLayout row = row();
        row.setBaselineAligned(false);
        addButtons(row, first, second);
        return row;
    }

    private void addButtons(LinearLayout row, Button first, Button second) {
        LinearLayout.LayoutParams left = new LinearLayout.LayoutParams(0, -2, 1);
        left.setMarginEnd(dp(8));
        row.addView(first, left);
        row.addView(second, new LinearLayout.LayoutParams(0, -2, 1));
    }

    private TextView text(int textId, Typeface font, int size, int colorId) {
        TextView view = new TextView(this);
        view.setTypeface(font);
        view.setTextSize(size);
        view.setTextColor(color(colorId));
        if (textId != 0) view.setText(textId);
        return view;
    }

    private TextView heading(int textId) {
        TextView view = text(textId, strong, 19, R.color.text);
        if (Build.VERSION.SDK_INT >= 28) view.setAccessibilityHeading(true);
        return view;
    }

    private TextView eyebrow(int textId, int colorId) {
        TextView view = text(textId, mono, 11, colorId);
        view.setLetterSpacing(0.11f);
        return view;
    }

    private LinearLayout labeledField(int labelId, EditText input) {
        LinearLayout group = column();
        TextView label = text(labelId, strong, 14, R.color.text);
        label.setLabelFor(input.getId());
        group.addView(label);
        add(group, input, 4);
        return group;
    }

    private EditText field(int id, int hint, boolean numeric) {
        EditText field = new EditText(this);
        field.setId(id);
        field.setInputType(numeric ? InputType.TYPE_CLASS_NUMBER
                : InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_URI | InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS);
        field.setSingleLine(true);
        field.setTypeface(mono);
        field.setTextSize(15);
        field.setTextColor(color(R.color.text));
        field.setHintTextColor(color(R.color.secondary));
        if (hint != 0) field.setHint(hint);
        field.setFilters(new InputFilter[] {new InputFilter.LengthFilter(numeric ? 5 : 253)});
        field.setMinimumHeight(dp(TOUCH_TARGET_DP));
        field.setPaddingRelative(dp(10), dp(8), dp(10), dp(8));
        field.setBackground(controlSurface(R.color.ground, R.color.field_line));
        field.setImportantForAutofill(View.IMPORTANT_FOR_AUTOFILL_NO);
        return field;
    }

    private Button button(int id, int textId, boolean primary, boolean caution, Runnable action) {
        Button button = new Button(this);
        button.setId(id);
        if (textId != 0) button.setText(textId);
        button.setAllCaps(false);
        button.setTypeface(strong);
        button.setTextSize(16);
        button.setMinWidth(0);
        button.setMinimumWidth(0);
        button.setMinHeight(dp(TOUCH_TARGET_DP));
        button.setMinimumHeight(dp(TOUCH_TARGET_DP));
        button.setPaddingRelative(dp(12), dp(7), dp(12), dp(7));
        button.setStateListAnimator(null);
        int ink = color(primary ? R.color.accent : caution ? R.color.caution : R.color.text);
        button.setTextColor(new ColorStateList(new int[][] { {-android.R.attr.state_enabled}, {} },
                new int[] { color(R.color.secondary), ink }));
        button.setBackground(controlSurface(
                primary ? R.color.accent_track : caution ? R.color.caution_track : R.color.track,
                primary ? R.color.accent : caution ? R.color.caution : R.color.field_line));
        button.setOnClickListener(view -> action.run());
        return button;
    }

    private RippleDrawable controlSurface(int fill, int edge) {
        StateListDrawable states = new StateListDrawable();
        states.addState(new int[] {-android.R.attr.state_enabled}, shape(color(R.color.track), color(R.color.line_strong), CONTROL_RADIUS_DP));
        states.addState(new int[] {android.R.attr.state_focused}, shape(color(fill), color(R.color.accent), CONTROL_RADIUS_DP, 2));
        states.addState(new int[] {android.R.attr.state_hovered}, shape(color(fill), color(R.color.text), CONTROL_RADIUS_DP));
        states.addState(new int[0], shape(color(fill), color(edge), CONTROL_RADIUS_DP));
        int ink = 0x30000000 | (color(R.color.accent) & 0x00FFFFFF);
        return new RippleDrawable(ColorStateList.valueOf(ink), states, null);
    }

    private GradientDrawable shape(int fill, int edge, int radius) {
        return shape(fill, edge, radius, 1);
    }

    private GradientDrawable shape(int fill, int edge, int radius, int stroke) {
        GradientDrawable background = new GradientDrawable();
        background.setColor(fill);
        background.setCornerRadius(dp(radius));
        if (edge != 0) background.setStroke(dp(stroke), edge);
        return background;
    }

    private int dp(int value) {
        return Math.round(value * getResources().getDisplayMetrics().density);
    }

    private int color(int resource) {
        return getColor(resource);
    }
}
