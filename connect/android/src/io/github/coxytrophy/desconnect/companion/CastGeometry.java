// SPDX-License-Identifier: MIT
package io.github.coxytrophy.desconnect.companion;

/** The projection region and default display report changes independently. */
final class CastGeometry {
    static final class Shape {
        final int width, height, rotation;
        Shape(int width, int height, int rotation) {
            this.width = width;
            this.height = height;
            this.rotation = rotation;
        }
        boolean same(Shape other) {
            return other != null && width == other.width && height == other.height && rotation == other.rotation;
        }
    }

    static final class Target {
        final int width, height;
        final Shape display;
        final boolean full;
        Target(int width, int height, Shape display, boolean full) {
            this.width = width;
            this.height = height;
            this.display = display;
            this.full = full;
        }
    }

    private final boolean legacyFullDisplay;
    private int capturedWidth, capturedHeight;
    private Target previous;

    CastGeometry(boolean legacyFullDisplay) {
        this.legacyFullDisplay = legacyFullDisplay;
    }

    Target capture(Shape display, int width, int height) {
        if (width < 1 || height < 1) return null;
        capturedWidth = width;
        capturedHeight = height;
        return display(display);
    }

    Target display(Shape display) {
        boolean reported = !legacyFullDisplay && capturedWidth > 0;
        int width = reported ? capturedWidth : display.width;
        int height = reported ? capturedHeight : display.height;
        boolean full = legacyFullDisplay || reported && width == display.width && height == display.height;
        if (previous != null && previous.width == width && previous.height == height
                && previous.full == full && previous.display.same(display)) return null;
        previous = new Target(width, height, display, full);
        return previous;
    }
}
