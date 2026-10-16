"""Builds a platform-specific, interpreter-independent wheel.

The wheel bundles the native msgkit library (loaded with ctypes), so it is
tagged ``py3-none-<platform>``: one wheel per OS/architecture, valid for every
Python 3 version. All metadata lives in pyproject.toml.
"""

import os

from setuptools import setup
from setuptools.dist import Distribution

try:  # setuptools >= 70.1
    from setuptools.command.bdist_wheel import bdist_wheel as _bdist_wheel
except ImportError:  # older setuptools with the separate `wheel` package
    from wheel.bdist_wheel import bdist_wheel as _bdist_wheel


class BinaryDistribution(Distribution):
    """Marks the distribution as containing platform-specific files."""

    def has_ext_modules(self):
        return True


class BDistWheel(_bdist_wheel):
    def finalize_options(self):
        super().finalize_options()
        self.root_is_pure = False

    def get_tag(self):
        _, _, plat = super().get_tag()
        # Release builds set MSGKIT_WHEEL_PLAT (e.g. manylinux_2_28_x86_64, macosx_11_0_arm64):
        # the host platform is not always the platform of the packaged library (cross-compiles).
        return "py3", "none", os.environ.get("MSGKIT_WHEEL_PLAT") or plat


setup(cmdclass={"bdist_wheel": BDistWheel}, distclass=BinaryDistribution)
