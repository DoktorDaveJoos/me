"""Synthetic payroll regression; never uses an actual person's document."""
from pathlib import Path
import json, subprocess, shutil
from reportlab.pdfgen import canvas
from reportlab.lib.utils import ImageReader
from PIL import Image

root = Path(__file__).parent / 'fixtures'
fields = [
 ('Name', 'Erika Beispiel'), ('Anschrift', 'Musterstrasse 12, 10115 Berlin'),
 ('Arbeitgeber', 'Musterwerk GmbH'), ('Personal-Nr.', '0004711'),
 ('Geburtsdatum', '01.02.1990'), ('Steuer-ID', '01234567890'),
 ('SV-Nummer', '12010290B123'), ('Eintritt', '01.03.2020'),
 ('Steuerklasse', '1'), ('Kinderfreibetrag', '0,0'),
 ('Krankenkasse', 'Techniker Krankenkasse'), ('Wochenarbeitszeit', '38,5 Stunden'),
 ('Grundgehalt', '4.350,00 EUR'), ('Zulage', '150,00 EUR'),
 ('Gesamt-Brutto', '4.500,00 EUR'), ('Steuer-Brutto', '4.500,00 EUR'),
 ('Lohnsteuer', '650,00 EUR'), ('Kirchensteuer', '0,00 EUR'),
 ('Solidaritaetszuschlag', '0,00 EUR'), ('KV Arbeitnehmer', '390,00 EUR'),
 ('RV Arbeitnehmer', '418,50 EUR'), ('AV Arbeitnehmer', '58,50 EUR'),
 ('PV Arbeitnehmer', '83,00 EUR'), ('Netto-Verdienst', '2.900,00 EUR'),
 ('Vorschuss / Netto-Abzug', '-100,00 EUR'), ('Auszahlungsbetrag', '2.800,00 EUR'),
 ('Bank', 'Musterbank'), ('IBAN', 'DE89 3704 0044 0532 0130 00'),
 ('BIC', 'COBADEFFXXX'), ('Jahresbrutto kumuliert', '36.000,00 EUR'),
]
(root/'payroll.json').write_text(json.dumps(dict(fields), ensure_ascii=False, indent=2)+'\n')
(root/'payroll.txt').write_text('SYNTHETISCHE GEHALTSABRECHNUNG\nAbrechnungszeitraum: August 2026\n'+ '\n'.join(f'{k}: {v}' for k,v in fields)+'\n')
pdf=root/'payroll.pdf'
c=canvas.Canvas(str(pdf), pagesize=(595,842), invariant=1)
c.setTitle('Synthetic payroll regression')
c.setFont('Helvetica-Bold',15);c.drawString(35,802,'SYNTHETISCHE GEHALTSABRECHNUNG')
c.setFont('Helvetica',11);c.drawString(35,780,'Abrechnungszeitraum: August 2026')
c.setFont('Helvetica',8);c.drawString(35,762,'Fiktive Testdaten - keine echte Gehaltsabrechnung')
y=735
for i,(label,value) in enumerate(fields):
 if i%2==0:
  c.setFillColorRGB(.95,.96,.97);c.rect(30,y-5,535,20,fill=1,stroke=0)
 c.setFillColorRGB(0,0,0);c.setFont('Helvetica',10)
 c.drawString(35,y,label);c.drawString(300,y,value);y-=20
c.save()
# Render the exact text PDF for the JPEG and scanned-PDF test variants.
subprocess.run([shutil.which('pdftoppm') or 'pdftoppm','-scale-to','2400','-singlefile','-png',str(pdf),'/tmp/me-payroll-preview'],check=True, stderr=subprocess.PIPE)
im=Image.open('/tmp/me-payroll-preview.png').convert('RGB')
im.save(root/'payroll.jpg',quality=96)
c=canvas.Canvas(str(root/'payroll-scan.pdf'),pagesize=(595,842),invariant=1)
c.drawImage(ImageReader(im),0,0,width=595,height=842);c.save()
print('Generated synthetic 30-field text PDF, scan PDF, JPEG and expected values.')
