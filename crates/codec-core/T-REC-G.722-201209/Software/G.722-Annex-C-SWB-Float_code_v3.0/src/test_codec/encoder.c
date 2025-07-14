/* ITU G.722 3rd Edition (2012-09) */

/*--------------------------------------------------------------------------
 ITU-T G.722 Annex C (ex G.722-SWB-Float) Source Code
 Software Release 1.00 (2012-05)
 (C) 2012 France Telecom, Huawei Technologies, VoiceAge Corp., NTT.
--------------------------------------------------------------------------*/

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <limits.h>
#include "pcmswb.h"
#include "softbit.h"

/***************************************************************************
* usage()
***************************************************************************/
static void usage(char progname[])
{
  fprintf(stderr, "\n");
  fprintf(stderr, " Usage: %s [-options] <infile> <codefile> <bitrate>\n", progname);
  fprintf(stderr, "\n");
  fprintf(stderr, " where:\n" );
  fprintf(stderr, "   infile       is the name of the input file to be encoded.\n");
  fprintf(stderr, "   codefile     is the name of the output bitstream file.\n");
  fprintf(stderr, "   bitrate      is the desired bitrate:\n");
  fprintf(stderr, "                 \"64\" (R1sm)              for G.722 core at 56 kbit/s,\n");
  fprintf(stderr, "                 \"80\" (R2sm), \"96\" (R3sm) for G.722 core at 64 kbit/s.\n");
  fprintf(stderr, "\n");
  fprintf(stderr, " Options:\n");
  fprintf(stderr, "   -quiet       quiet processing.\n");
  fprintf(stderr, "\n");
}

typedef struct {
  int  mode;
  int  quiet;
  int  format;
  unsigned short  inputSF;
  char *input_fname;
  char *code_fname;
} ENCODER_PARAMS;

static void  get_commandline_params(
                                    int            argc,
                                    char           *argv[],
                                    ENCODER_PARAMS *params
                                    ) 
{
  char  *progname=argv[0];

  if (argc < 4) {
    fprintf(stderr, "Error: Too few arguments.\n");
    usage(progname);
    exit(1);
  }

  /* Default mode */
  params->mode = -1;
  params->quiet = 0;
  params->format = 0;        /* Default is G.192 softbit format */
  params->inputSF = 32000;   /* Default is super-wideband input */

  /* Search options */
  while (argc > 1 && argv[1][0] == '-') {
    if (strcmp(argv[1],"-quiet") == 0) {
      /* Set the quiet mode flag */
      params->quiet=1;
      /* Move arg{c,v} over the option to the next argument */
      argc--;
      argv++;
    }
    else if (strcmp(argv[1], "-h") == 0 || strcmp(argv[1], "-?") == 0) {
      /* Display help message */
      usage(progname);
      exit(1);
    }
    else {
      fprintf(stderr, "Error: Invalid option \"%s\"\n\n",argv[1]);
      usage(progname);
      exit(1);
    }
  }

  /* Open input signal and output code files. */
  params->input_fname  = argv[1];
  params->code_fname   = argv[2];

  /* bitrate */
  if (strcmp(argv[3], "64") == 0) {
    params->mode = MODE_R1sm;
  }
  else if (strcmp(argv[3], "80") == 0) {
    params->mode = MODE_R2sm;
  }
  else if (strcmp(argv[3], "96") == 0) {
    params->mode = MODE_R3sm;
  }
  else {
    fprintf(stderr, "Error: Invalid bitrate number %s\n", argv[3]);
    fprintf(stderr, "                           \"64\"         for G.722 core at 56 kbit/s,\n");
    fprintf(stderr, "                           \"96\" or \"80\" for G.722 core at 64 kbit/s.\n");
    usage(progname);
    exit(-1);
  }

  /* check for core/mode compatibility */
  switch (params->mode) 
  {
    case MODE_R00wm : break;
    case MODE_R0wm  : break;
    case MODE_R1wm  : break;
    case MODE_R1sm  : break;
    case MODE_R2sm  : break;
    case MODE_R3sm  : break;
    default : fprintf(stderr, "Error: Inconsitency in core and bitrate.\n");
    usage(progname); exit(-1);
  }

  return;
}

/***************************************************************************
* main()
***************************************************************************/

int
main(int argc, char *argv[])
{
  int             i;
  ENCODER_PARAMS  params;
  int             nsamplesIn;
  int             nbitsOut;
  int             nbytesOut;
  FILE            *fpin, *fpcode;

  void            *theEncoder=0;

  int             status;
  short           sbufIn[NSamplesPerFrame32k];
  unsigned short  sbufOut[G192_HeaderSize+MaxBitsPerFrame];
  unsigned char   cbufOut[MaxBytesPerFrame];

  /* Set parameters from argv[]. */
  get_commandline_params( argc, argv, &params );

  if ( params.inputSF == 8000 )
    nsamplesIn = NSamplesPerFrame08k; /* Input sampling rate is 8 kHz. */
  else if ( params.inputSF == 16000 )
    nsamplesIn = NSamplesPerFrame16k; /* Input sampling rate is 16 kHz. */
  else 
    nsamplesIn = NSamplesPerFrame32k; /* Input sampling rate is 32 kHz in default. */

  switch (params.mode) {
    case MODE_R00wm : nbitsOut = NBITS_MODE_R00wm; break;
    case MODE_R0wm  : nbitsOut = NBITS_MODE_R0wm;  break;
    case MODE_R1wm  : nbitsOut = NBITS_MODE_R1wm;  break;
    case MODE_R1sm  : nbitsOut = NBITS_MODE_R1sm;  break;
    case MODE_R2sm  : nbitsOut = NBITS_MODE_R2sm;  break;
    case MODE_R3sm  : nbitsOut = NBITS_MODE_R3sm;  break;
    default : fprintf(stderr, "Mode specification error.\n"); exit(-1);
  }
  nbytesOut = nbitsOut/CHAR_BIT;

  /* Open input speech file. */
  fpin = fopen(params.input_fname, "rb");
  if (fpin == (FILE *)NULL) {
    fprintf(stderr, "file open error.\n");
    exit(1);
  }

  /* Open output bitstream. */
  fpcode = fopen(params.code_fname, "wb");
  if (fpcode == (FILE *)NULL) {
    fprintf(stderr, "file open error.\n");
    exit(1);
  }

  /* Instanciate an encoder. */
  theEncoder = pcmswbEncode_const(params.inputSF, params.mode);

  if (theEncoder == 0) {
    fprintf(stderr, "Encoder init error.\n");
    exit(1);
  }

  /* Reset (unnecessary if right after instantiation!). */
  pcmswbEncode_reset( theEncoder );

  while (1) {
    /* Initialize sbuf[]. */
    for (i=0; i<nsamplesIn; i++) sbufIn[i] = 0;

    /* Read input singal from fin. */
    if ( fread( sbufIn, sizeof(short), nsamplesIn, fpin ) == 0 )
      break;

    /* Encode. */
    status = pcmswbEncode( sbufIn, cbufOut, theEncoder );

    if ( status ) {
      fprintf(stderr, "Encoder NG. Exiting.\n");
      exit(1);
    }

    if( params.format == 0 ) {   /* G.192 softbit output format */
      /* Write main header */
      sbufOut[0] = G192_SYNCHEADER;
      sbufOut[idxG192_BitstreamLength] = (unsigned short)nbitsOut;

      /* Convert from hardbit to softbit. */
      hardbit2softbit( nbytesOut, cbufOut, &sbufOut[G192_HeaderSize] );

      /* Write bitstream. */
      fwrite( sbufOut, sizeof(short), G192_HeaderSize+nbitsOut, fpcode );
    }
    else {   /* Hardbit output format */
      /* Write bitstream. */
      fwrite( cbufOut, sizeof(char), nbytesOut, fpcode );
    }
  }

  /* Close files. */
  fclose(fpin);
  fclose(fpcode);

  /* Delete the encoder. */
  pcmswbEncode_dest( theEncoder );

  return 0;
}
