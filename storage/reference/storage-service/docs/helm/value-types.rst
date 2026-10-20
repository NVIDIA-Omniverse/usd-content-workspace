Value types used in configuration
=================================

Aside from built-in YAML types, such as string, numbers, booleans, and so on, Storage Service uses special types that require custom formatting.

Duration
--------

``Duration`` is a string type used to represent time spans. It comprises a numeric value followed by a unit suffix, for example: ``1h`` or ``3600s``. You can use any of the following suffixes:

* ``ns`` - for nanoseconds,
* ``us`` - for microseconds,
* ``s`` - for seconds,
* ``m`` - for minutes,
* ``h`` - for hours,
* ``d`` - for days.

Size
----

``Size`` is a string type used to represent sizes of an object, in bytes. It comprises a numeric value followed by a unit suffix, for example: ``10MiB`` or ``10240KB``. You can use any of the following suffixes:

* ``B`` - for single bytes,
* ``KB`` - for 10<sup>3</sup> bytes,
* ``KiB`` - for 2<sup>10</sup> bytes,
* ``MB`` - for 10<sup>6</sup> bytes,
* ``MiB`` - for 2<sup>20</sup> bytes,
* ``GB`` - for 10<sup>9</sup> bytes,
* ``GiB`` - for 2<sup>30</sup> bytes,
* ``TB`` - for 10<sup>12</sup> bytes,
* ``TiB`` - for 2<sup>40</sup> bytes.
