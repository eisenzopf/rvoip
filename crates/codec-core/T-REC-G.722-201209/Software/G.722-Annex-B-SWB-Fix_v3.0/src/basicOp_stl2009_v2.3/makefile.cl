# Makefile for Microsoft Visual C++: nmake -f makefile.cl
.SUFFIXES: .obj .c .cpp

CC=cl

INCLUDES=

DFLAGS= -DWMOPS=1

CFLAGS= -nologo -G5 -GX -O2 -W3 $(INCLUDES) $(DFLAGS)

# Do not change the library name.
LIBRARY= basop.lib

# List header files.
HDRS= basop32.h \
      control.h \
      count.h   \
      enh1632.h \
      move.h    \
      patch.h   \
      stl.h     \
      typedef.h \
      typedefs.h
#     enh40.h

# List object files.
OBJS= basop32.obj \
      control.obj \
      count.obj   \
      enh1632.obj
#     enh40.obj

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
