# Makefile for Microsoft Visual C++: nmake -f makefile.cl
.SUFFIXES: .obj .c .cpp

CC=cl

INCLUDES= -I../basicOp_stl2009_v2.3 -I../mainlib/util

DFLAGS= -DWMOPS=1

CFLAGS= -nologo -G5 -GX -O2 -W3 $(INCLUDES) $(DFLAGS)

# Do not change the library name.
LIBRARY= basic_math.lib

# List header files.
HDRS= log2.h \
      math_op.h \
      oper_32b.h \

# List object files.
OBJS= log2.obj \
      math_op.obj \
      oper_32b.obj \

all: $(LIBRARY)

$(LIBRARY): $(OBJS)
    del $(LIBRARY)
    lib -nologo -OUT:$(LIBRARY) $(OBJS)

$(OBJS): $(HDRS)

clean:
    del /S *.obj *.exe

.c.obj:
    $(CC) -c $(CFLAGS) $*.c -Fo$*.obj
.cpp.obj:
    $(CC) -c $(CFLAGS) $*.cpp -Fo$*.obj
