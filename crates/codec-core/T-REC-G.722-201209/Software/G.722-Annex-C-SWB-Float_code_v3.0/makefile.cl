# Makefile for Microsoft Visual C++: nmake -f makefile.cl
.SUFFIXES: .obj .c .cpp

MAKE=nmake -f makefile.cl

SUBDIRS=src\mainlib src\test_codec

TEST_DIR=src\test_codec
OUT_DIR=out

all: $(SUBDIRS)

$(SUBDIRS)::
    cd $@
    $(MAKE)
    cd ..\..

clean:
    del /S *.obj *.lib
    del $(TEST_DIR)\*.exe
    del /Q $(OUT_DIR)
    rmdir $(OUT_DIR)

test:
    test test

proc:
    test proc
    
comp:
    test comp
