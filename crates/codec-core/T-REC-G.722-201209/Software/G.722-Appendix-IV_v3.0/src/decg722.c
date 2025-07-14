/* ITU G.722 3rd Edition (2012-09) */

/* ITU-T G722 PLC Appendix IV - Reference C code for fixed-point implementation */
/* Version:       1.2                                                           */
/* Revision Date: Jul 3, 2007                                                   */

/*
   ITU-T G.722 PLC Appendix IV   ANSI-C Source Code
   Copyright (c) 2006-2007
   France Telecom
*/


/* Copyright and version information from the original G.722 header:
============================================================================

DECG722.C 
~~~~~~~~~

Original author:
~~~~~~~~~~~~~~~~
J-P PETIT 
CNET - Centre Lannion A
LAA-TSS                         Tel: +33-96-05-39-41
Route de Tregastel - BP 40      Fax: +33-96-05-13-16
F-22301 Lannion CEDEX           Email: petitjp@lannion.cnet.fr
FRANCE

History:
~~~~~~~~
14.Mar.95  v1.0       Released for use ITU-T UGST software package Tool
                      based on the CNET's 07/01/90 version 2.00
01.Jul.95  v2.0       Changed function declarations to work with many compilers;
                      reformated <simao@ctd.comsat.com>
22.May.06  v2.3       Updated with g192 format reading and basic index domain PLC functionality
                      <{nicklas.sandgren,jonas.svedberg}@ericsson.com>
23.Aug.06  v3.0 beta  Updated with STL2005 v2.2 basic operators and G.729.1 methodology
                      <{balazs.kovesi,stephane.ragot}@orange-ft.com>
============================================================================
*/

/* Standard prototypes */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>

/* G.722- and UGST-specific prototypes */
#include "g722.h"
#include "ugstdemo.h"
#include "g722_com.h"
#include "g722_plc.h"
#include "stl.h"

/*******************
 * local routines  *
 *******************/

/* declarations */

short g192_to_byte(short nb_bits, short *code_byte, short *inp_bits, short N_samples);
void verify_bitstream(FILE* F_cod, char* FileIn);
void display_usage ARGS((void));

/* definitions */

short g192_to_byte(short nb_bits, short *code_byte, short *inp_bits, short N_samples){
   long i,j,k;
   short bit,mask,mode=-1;
   for(i=0; i<N_samples/2; i++){
     code_byte[i]=0;
   }

   /* convert soft bits value [-127 ... +127] range to hard bits (G192_ONE or G192_ZERO) */
   for(i=0;i < nb_bits;i++){      
      if(inp_bits[i]&0x0080){ /* look at sign bit only */
         inp_bits[i] = G192_ONE;
      } else {
         inp_bits[i] = G192_ZERO;
      }
   } 

   if(nb_bits==(N_samples/2)*8) { mode=1;};
   if(nb_bits==(N_samples/2)*7) { mode=2;};
   if(nb_bits==(N_samples/2)*6) { mode=3;};

   if(nb_bits==0){
      mode = -1; /* flag for NO_DATA or BAD SPEECH frame, without payload */
   } else {
      /* special ordering for scalability reasons */
      /* write [ b2*n, b3*n, b4*n, b5*n, b6*n, b7*n, b1*n, b0*n]  to enable truncation of G.722 g192 frames */ 
      /*  "core"/non-scalable layer is read first b2s-b3s-b4s-b5s */
      /* b6s,b7s are semi scalable, (b6,b7) scalability is not part of the g.722 standard */
      j=0;
      for (bit=2;bit<8;bit++){
         mask=(0x0001<<(bit));
         for(i=0; i < (N_samples/2); i++,j++){
            if(inp_bits[j] == G192_ONE){
               code_byte[i] |= mask; /* set bit */  
            } 
            /* leave bit zero */
         }
      }

      /* embedded layers last in G.192 frame */
      /* read b1s followed by b0s if available */
      k=0;                /* 64 kbps */
      if(mode==2) {k=1;}; /* 56 kbps */
      if(mode==3) {k=2;}; /* 48 kbps*/
      for (bit=1;bit>=k;bit--){
         mask=(0x0001<<(bit)); 
         for(i=0; i < (N_samples/2); i++,j++){
            if(inp_bits[j] == G192_ONE){
               code_byte[i] |= mask;
            } 
         }
      }
   }
   return mode;
}

void verify_bitstream(FILE* F_cod, char* FileIn)
{
   long  i;
   char  tmp_type;   /* for input type checking*/
   char  bs_format = g192;
   short header[2];  /* g192 header size*/

	/* Do preliminary inspection in the INPUT BITSTREAM FILE to check
       that it has a correct format (g192) */
	i = check_eid_format(F_cod, FileIn, &tmp_type);

	/* Check whether the  required input format matches with the one in the file */
	if (i != bs_format)
	{
		/* The input bitstream format is not g192 */
		fprintf (stderr, "*** Illegal input bitstream format: %s (should be %s) ***\n",
				 format_str(i),format_str((int)bs_format));
		HARAKIRI("\nExiting...\n\n",1);
	}
	{
		/* check input file for valid initial G.192 synchronism headers */
		short sync_header=0;
        fread(header, sizeof(short), 2, F_cod); /* Get presumed first G.192 sync header */
        i = header[1]; /* header[1] should have the frame length */

        /* advance file to the (presumed) next G.192 sync header */
        fseek(F_cod, (long)(header[1])*sizeof(short), SEEK_CUR);
        fread(header, sizeof(short), 2, F_cod);  /* get (presumed) next G.192 sync header */

		if ((header[0] & 0xFFF0) == (G192_FER & 0xFFF0))
		{ /* Verify */
			sync_header = 1;
        }    
        fseek(F_cod, 0l, SEEK_SET); /* Rewind BP file */
        if(sync_header==0){
            HARAKIRI("Error::Input bitstream MUST have valid G192 sync_headers \n\n",1);
        }
	}
}

void display_usage()
{
   /* Quit program */
   printf("USAGE: \n\n");
   printf("  decg722 [-fsize N] g192_bst output\n\n");
   printf("  where N is frame size at 16 kHz (default: %d)\n", DEF_FR_SIZE);
   printf("\n\n");

   exit(-128);
}



/*********
 * MAIN  *
 *********/

int main (int argc, char * argv[])
{
	short code[MAX_BYTESTREAM_BUFFER]; 	    /* byte stream buffer , 1 byte(8 bit) at	8 kHz */
	short outcode[MAX_OUTPUT_SP_BUFFER];    /* speech buffer */
  short	inp_frame[2+MAX_BITLEN_SIZE];     /* g192 inpFrame , 1 bit is stored per 16 bit Word*/
  long 	frames=1;                         /* frame counter */
  char  FileIn[MAX_STR], FileOut[MAX_STR];
  FILE  *F_cod, *F_out;
  long 	iter=0;
  long 	N=DEF_FR_SIZE, smpno=0;
  long 	i=0;
  short	header[2];                         /* g192 header size*/
  long 	fr_size[N_MODES+1];                /* analysis frame sizes in bits for every mode */
  short	mode_frame;                        /* mode for the current input frame */
  g722_state 	decoder;
  void 	      * plc_state; 			           /* FEC states */
#ifdef WMOPS
  short spe1Id = -1;
  short spe2Id = -1;
  Word32 tot_wm1; 
  Word16 num_frames1;
  Word32 tot_wm2; 
  Word16 num_frames2;
#endif

   /* print Message */
   printf("\n");
   printf("              ***** ITU-T G.722 Appendix IV *****\n");
   printf("\n");
   printf("                Fixed-point C simulation\n");
   printf("                Version 1.1 - Jul 3, 2007\n");
   printf("\n");
   printf("                COPYRIGHT France Telecom R&D\n");

   printf("\n *************************************************************");
   printf("\n *  SIMULATION BASED ON ITU-T G.722 DECODER V3.0b            *");
   printf("\n *  WITH G.192 BITSTREAM FORMAT AND STL2005 BASIC OPERATORS  *");
   printf("\n *       Copyright CNET LANNION A TSS/CMC Aug 24, 1990       *");
   printf("\n *       Copyright Ericsson AB.           May 22, 2006       *");
   printf("\n *       Copyright France Telecom R&D     Aug 23, 2006       *");
   printf("\n *************************************************************");
   printf("\n\n");

  /*********************************
   * handle command line arguments *
   *********************************/

  if (argc < 2)
    display_usage();
  else
  {
    while (argc > 1 && argv[1][0] == '-')
      if (strcmp(argv[1], "-fsize") == 0)
      {
        /* Define frame size for g192 operation and file reading */
        N = atoi(argv[2]);
        if(( N != 160) && (N != 320)) {
          fprintf(stderr, "ERROR! Invalid frame size \"%s\" in command line, only 160 or 320 is supported\n\n",argv[2]);
          display_usage();
        }
        /* Move argv over the option to the next argument */
        argv+=2;
        argc-=2;
      } else {
        fprintf(stderr, "ERROR! Invalid option \"%s\" in command line\n\n", argv[1]);
        display_usage();
      }
  }

  /* print arguments */   
  GET_PAR_S(1, "Input file : ", FileIn);
  GET_PAR_S(2, "Output file: ", FileOut);
  printf(      "Frame size : %ld\n\n", N);

  /* Open input file */
  if ((F_cod = fopen (FileIn, RB)) == NULL){
    KILL(FileIn, -2);
  }

  /* Open output file */
  if ((F_out = fopen (FileOut, WB)) == NULL) {
    KILL(FileOut, -2);
  }

  /**************************
   * decoder initialization *
   **************************/

  /* set frame sizes in bits for the possibe  modes(1(64kbps),2(56 kbps),3(48 kbps)) */
  for(i=1;i<(N_MODES+1);i++){
    fr_size[i]=(N/2)*(8+1-i);
  }
  fr_size[0] = -1; /* unused entry */

  /* Reset lower and upper band encoders */
  g722_reset_decoder(&decoder);
  plc_state = G722PLC_init((short)(N/2)); /* N/2 as low band */

  /*****************************
   * initialize WMOPS counters *
   *****************************/

#if (WMOPS)
  setFrameRate(16000, N);
  spe1Id = getCounterId("Bad frames");
  setCounter(spe1Id);
  Init_WMOPS_counter();
  spe2Id = getCounterId("Good frames");
  setCounter(spe2Id);
  Init_WMOPS_counter();
#endif

  /********************
   * verify bitstream *
   ********************/
  verify_bitstream(F_cod, FileIn);

  /**********
   * decode *
   **********/

  /* loop over G.192 frames */
  while (fread (header, sizeof (short), 2, F_cod) == 2)
  {
    /* check header */
		if(!((short)fread (inp_frame, sizeof (short), header[1], F_cod) == header[1]))
    {
			HARAKIRI("Error::Could not read complete frame of input bitstream\n\n",1);
		} else if (!((header[1]==0) || (header[1]==fr_size[1]) || (header[1]==fr_size[2])  || (header[1]==fr_size[3])))
    {
			HARAKIRI("Error::invalid input frame size in bitstream, only (0,48,56,64) kbps are allowed)\n\n",1);
		} else
    {          /* normal decoding */
      fprintf(stderr, "frame=%ld\r", frames);
    }

    /* convert G.192 bistream to byte format          -> code
			 set mode for G.722 decoder (48, 56, 64 kbit/s) -> mode_frame */
    mode_frame = g192_to_byte(header[1],code,inp_frame,(short)N);

    /* decode frame */
    if((header[0] != G192_SYNC) || (mode_frame == -1))
    { /*------ decode bad frame ------*/ 
#if (WMOPS)
      /* set WMOPS counter to "PLC" */
      setCounter(spe1Id);
      fwc();
      Reset_WMOPS_counter();
#endif

      /* extrapolate missing frame */
      smpno = G722PLC_conceal(plc_state, outcode, &decoder);

      /* set previous bfi to bad frame */
      ((G722PLC_STATE *)plc_state)->prev_bfi = 1;
#if (WMOPS)
      move16();
#endif
    }
    else
    { /*------ decode good frame ------*/
#if (WMOPS)
      /* set WMOPS counter to "good frame decoding" */
      setCounter(spe2Id);
      fwc();
      Reset_WMOPS_counter();
#endif
                
       /* G.722 decoder (including cross-fade after bad frame and PLC memory update) */
       smpno = G722PLC_decode(code, outcode, mode_frame, (N/2), &decoder, plc_state);

       /* set previous bfi to good frame */
       ((G722PLC_STATE *)plc_state)->prev_bfi = 0;
#if (WMOPS)
        move16();
#endif
    }

    /* update sample and frame counters */
    iter += smpno;
    frames++;

    /* write decoded speech samples */
    if ((long)fwrite ((char *) outcode, sizeof (Word16), N, F_out) != N)
    {
      KILL(FileOut,-4);
    }
	}

	/***************
   * print WMOPS *
   ***************/
#if (WMOPS)
  setCounter(spe1Id);
	fwc();
	WMOPS_output_avg(0, &tot_wm1, &num_frames1);
	setCounter(spe2Id);
	fwc();
	WMOPS_output_avg(0, &tot_wm2, &num_frames2);
  printf("Global average %.3f WMOPS\n", (tot_wm1 + tot_wm2)/(float)(num_frames1 + num_frames2));
  printf("\n\n");
#endif

	/*************************
   * clear memory and exit *
   *************************/
	G722PLC_clear(plc_state);
	fclose(F_out);
	fclose(F_cod);
	
	return (0);
}
