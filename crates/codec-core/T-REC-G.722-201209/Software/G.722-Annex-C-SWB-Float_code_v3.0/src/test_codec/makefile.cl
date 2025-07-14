# Makefile for Microsoft Visual C++: nmake -f makefile.cl
.SUFFIXES: .obj .c

CC=cl


INCLUDES= -I../mainlib/pcmswb

DFLAGS=

CFLAGS= -nologo -EHsc -O2 -W3 $(INCLUDES) $(DFLAGS)

LIBS=../mainlib/pcmswb.lib

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