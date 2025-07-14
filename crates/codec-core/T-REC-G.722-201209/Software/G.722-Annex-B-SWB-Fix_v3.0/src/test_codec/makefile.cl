# Makefile for Microsoft Visual C++: nmake -f makefile.cl
.SUFFIXES: .obj .c

CC=cl


INCLUDES= -I../basic_math -I../basicOp_stl2009_v2.3 -I../mainlib/pcmswb -I../drcnt

DFLAGS= -DWMOPS=1 \
#        -DDYN_RAM_CNT \
        -DSUPPRESS_COUNTER_RESULTS \

CFLAGS= -nologo -G5 -GX -O2 -W3 $(INCLUDES) $(DFLAGS)

LIBS=../mainlib/pcmswb.lib ../basic_math/basic_math.lib ../basicOp_stl2009_v2.3/basop.lib

HDRS=

OBJS1=encoder.obj

OBJS2=decoder.obj

PROGRAM1=encoder.exe

PROGRAM2=decoder.exe

all: $(PROGRAM1) $(PROGRAM2)

$(PROGRAM1): $(OBJS1) $(LIBS)
    $(CC) $(CFLAGS) -Fe$(PROGRAM1) $(OBJS1) $(LIBS)

$(PROGRAM2): $(OBJS2) $(LIBS)
    $(CC) $(CFLAGS) -Fe$(PROGRAM2) $(OBJS2) $(LIBS)

$(OBJS1): $(HDRS)

$(OBJS2): $(HDRS)

clean:
    del *.obj *.exe

.c.obj:
    $(CC) -c $(CFLAGS) $*.c -Fo$*.obj
.cpp.obj:
    $(CC) -c $(CFLAGS) $*.cpp -Fo$*.obj